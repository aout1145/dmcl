//! 任务运行上下文：日志、取消、动态节点与回滚登记。

use std::error::Error;
use std::fmt;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::watch;

use crate::bus::EventBus;
use crate::deps::{Deps, DynamicHandle, NodeSlot};
use crate::event::{TaskEvent, TaskRef};
use crate::graph::StepId;
use crate::progress::ProgressHandle;
use crate::runtime::Runtime;
use crate::task::Task;
use crate::warning::{WarningDecision, WarningGate};

/// 任务被取消时返回的错误，调用者可以用 `downcast_ref` 识别它。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cancelled;

impl fmt::Display for Cancelled {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("任务已取消")
    }
}

impl Error for Cancelled {}

struct CancelState {
    flag: AtomicBool,
    tx: watch::Sender<bool>,
}

impl fmt::Debug for CancelState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CancelState")
            .field("cancelled", &self.flag.load(Ordering::SeqCst))
            .finish()
    }
}

/// 取消句柄：可从任意线程（含 GUI 的事件循环）触发取消。
#[derive(Debug, Clone)]
pub struct CancelHandle {
    state: Arc<CancelState>,
}

impl Default for CancelHandle {
    fn default() -> Self {
        let (tx, _rx) = watch::channel(false);
        Self {
            state: Arc::new(CancelState {
                flag: AtomicBool::new(false),
                tx,
            }),
        }
    }
}

impl CancelHandle {
    /// 请求取消。已经取消过则无副作用。
    pub fn cancel(&self) {
        self.state.flag.store(true, Ordering::SeqCst);
        let _ = self.state.tx.send(true);
    }

    pub fn is_cancelled(&self) -> bool {
        self.state.flag.load(Ordering::SeqCst)
    }

    /// 已被取消则返回 [`Cancelled`] 错误，否则 `Ok(())`。
    pub fn check_cancelled(&self) -> Result<()> {
        if self.is_cancelled() {
            Err(anyhow::Error::new(Cancelled))
        } else {
            Ok(())
        }
    }

    /// 等待取消发生；若已经取消则立即返回。
    pub async fn cancelled(&self) {
        if self.is_cancelled() {
            return;
        }
        let mut rx = self.state.tx.subscribe();
        if self.is_cancelled() {
            return;
        }
        let _ = rx.changed().await;
    }
}

/// 向动态节点追加失败时返回的错误。
#[derive(Debug)]
pub enum AddTaskError {
    /// 图已经结束 / 取消，或创建者已经结束，不能再追加动态节点。
    GraphClosed,
    /// 动态句柄不能作为依赖传给别的节点。
    DynamicDependencyNotAllowed { task: TaskRef },
    /// 动态节点的依赖会与创建者形成环。
    Cycle { task: TaskRef },
}

impl fmt::Display for AddTaskError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AddTaskError::GraphClosed => f.write_str("任务图已经结束，不能再追加动态任务"),
            AddTaskError::DynamicDependencyNotAllowed { task } => {
                write!(f, "动态任务「{}」不能作为其他节点的依赖", task.name())
            }
            AddTaskError::Cycle { task } => {
                write!(f, "动态任务「{}」的依赖会与创建者形成环", task.name())
            }
        }
    }
}

impl Error for AddTaskError {}

/// 节点失败 / 取消时要执行的回滚动作。
#[async_trait::async_trait]
pub(crate) trait Cleanup: Send + Sync {
    async fn run(&self, ctx: TaskContext);
}

pub(crate) struct FnCleanup<F>(pub(crate) F);

#[async_trait::async_trait]
impl<F, Fut> Cleanup for FnCleanup<F>
where
    F: Fn(TaskContext) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    async fn run(&self, ctx: TaskContext) {
        (self.0)(ctx).await;
    }
}

/// 交给每个任务的运行上下文。
#[derive(Clone)]
pub struct TaskContext {
    runtime: Arc<Runtime>,
    node: usize,
    slot: Arc<NodeSlot>,
    bus: Arc<EventBus>,
    cancel: CancelHandle,
    progress: ProgressHandle,
    name: Arc<str>,
}

impl TaskContext {
    pub(crate) fn graph(runtime: Arc<Runtime>, node: usize, slot: Arc<NodeSlot>) -> Self {
        Self {
            runtime: runtime.clone(),
            node,
            slot: slot.clone(),
            bus: runtime.bus.clone(),
            cancel: runtime.cancel.clone(),
            progress: runtime.progress.clone(),
            name: slot.name.clone(),
        }
    }

    /// 当前任务节点的显示名。
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn cancel_handle(&self) -> CancelHandle {
        self.cancel.clone()
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// 已被取消则返回 [`Cancelled`] 错误，否则 `Ok(())`。
    pub fn check_cancelled(&self) -> Result<()> {
        self.cancel.check_cancelled()
    }

    /// 整个任务图共享的下载进度。
    pub fn progress_handle(&self) -> ProgressHandle {
        self.progress.clone()
    }

    /// 当前节点所属的显示步骤。
    pub fn step(&self) -> StepId {
        self.slot.step
    }

    /// 上报一条**日志**：一次性文字。
    pub fn log(&self, message: impl Into<String>) {
        let _ = self.bus.publish(TaskEvent::Log {
            task: Some(self.task()),
            message: message.into(),
        });
    }

    /// 上报一条需要前端决策的**警告**，并阻塞等待答复。
    ///
    /// * 没有订阅者时直接继续；
    /// * 前端 [`WarningDecision::abort`] → 全局取消，返回 [`Cancelled`]；
    /// * 前端 [`WarningDecision::accept`] → 继续。
    ///
    /// 等待期间只阻塞当前节点，其他节点继续调度。
    pub async fn confirm(&self, message: impl Into<String>) -> Result<()> {
        let gate = WarningGate::new();
        let delivered = self.bus.publish(TaskEvent::Warning {
            task: Some(self.task()),
            message: message.into(),
            decision: WarningDecision(gate.clone()),
        });
        if delivered {
            tokio::select! {
                accepted = gate.wait() => {
                    if !accepted {
                        self.cancel.cancel();
                    }
                }
                _ = self.cancel.cancelled() => return self.check_cancelled(),
            }
        }
        self.check_cancelled()
    }

    /// 在**当前节点**登记一个回滚动作。
    ///
    /// 节点成功完成时这些动作不会被删除，而是交给整张图；图最终失败 / 取消时按节点
    /// 完成顺序的逆序执行。节点自身失败 / 取消时立即按注册逆序执行。
    ///
    /// 只允许在节点运行期间注册；节点结束后再注册会被忽略并记一条日志。
    pub fn on_rollback<F, Fut>(&self, cleanup: F)
    where
        F: Fn(TaskContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        if !self
            .runtime
            .register_cleanup(self.node, Arc::new(FnCleanup(cleanup)))
        {
            self.log("任务已结束，回滚动作注册被忽略");
        }
    }

    /// 在执行期追加一个动态节点。
    ///
    /// 动态节点自动成为创建者的依赖：创建者要等它（及其全部动态子节点）结束才会
    /// 结束。需要返回值时才保留返回的 [`DynamicHandle`] 并调用 `wait()`。
    pub fn add_task<D, T, O>(
        &self,
        step: StepId,
        name: impl Into<String>,
        deps: D,
        task: T,
    ) -> Result<DynamicHandle<O>, AddTaskError>
    where
        D: Deps,
        T: Task<D::Output, O> + 'static,
        O: Send + Sync + 'static,
    {
        self.runtime.add_task(self.node, step, name, deps, task)
    }

    fn task(&self) -> TaskRef {
        self.slot.task_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_handle_is_shared_between_clones() {
        let handle = CancelHandle::default();
        let clone = handle.clone();
        assert!(!handle.is_cancelled());
        clone.cancel();
        assert!(handle.is_cancelled());
    }

    #[tokio::test]
    async fn waiting_for_cancel_returns_immediately_when_already_cancelled() {
        let handle = CancelHandle::default();
        handle.cancel();
        tokio::time::timeout(std::time::Duration::from_secs(1), handle.cancelled())
            .await
            .expect("already cancelled should not block");
    }

    #[tokio::test]
    async fn waiting_for_cancel_wakes_up_on_signal() {
        let handle = CancelHandle::default();
        let waiter = handle.clone();
        let task = tokio::spawn(async move { waiter.cancelled().await });
        tokio::task::yield_now().await;
        handle.cancel();
        tokio::time::timeout(std::time::Duration::from_secs(1), task)
            .await
            .expect("should wake up")
            .unwrap();
    }
}
