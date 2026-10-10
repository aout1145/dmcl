//! 任务运行上下文：进度上报 + 取消。

use std::error::Error;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::{broadcast, watch};

use crate::event::TaskEvent;
use crate::progress::Progress;

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
        // `subscribe` 之后再检查一次，避免"检查与订阅之间"丢失通知。
        if self.is_cancelled() {
            return;
        }
        let _ = rx.changed().await;
    }
}

/// 交给每个任务的运行上下文。
///
/// 任务通过它上报进度、检查取消状态；它**不持有**任何全局状态，
/// 因此多个任务链可以并发运行而互不干扰。
#[derive(Clone)]
pub struct TaskContext {
    events: broadcast::Sender<TaskEvent>,
    cancel: CancelHandle,
    step: usize,
    steps: usize,
    name: Arc<str>,
}

impl TaskContext {
    pub(crate) fn new(
        events: broadcast::Sender<TaskEvent>,
        cancel: CancelHandle,
        step: usize,
        steps: usize,
        name: impl Into<Arc<str>>,
    ) -> Self {
        Self {
            events,
            cancel,
            step,
            steps,
            name: name.into(),
        }
    }

    /// 由当前上下文派生一个子上下文：事件流向同一个订阅者，但带上子任务的名字
    /// 与父步骤的下标。任务内部再运行别的任务时用它，进度就不会串台。
    pub fn child(&self, name: impl Into<Arc<str>>) -> Self {
        Self {
            events: self.events.clone(),
            cancel: self.cancel.clone(),
            step: self.step,
            steps: self.steps,
            name: name.into(),
        }
    }

    /// 当前上下文的名字（步骤名或子任务名）。
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
        if self.is_cancelled() {
            Err(anyhow::Error::new(Cancelled))
        } else {
            Ok(())
        }
    }

    /// 上报一条进度。
    pub fn report(&self, progress: Progress) {
        let _ = self.events.send(TaskEvent::Progress {
            step: self.step,
            steps: self.steps,
            name: self.name.to_string(),
            progress,
        });
    }

    /// 上报一条只要文字的进度（总量未知）。
    pub fn status(&self, message: impl Into<String>) {
        self.report(Progress::status(message));
    }

    /// 上报一条带工作量的进度。
    pub fn progress(&self, current: u64, total: Option<u64>, message: impl Into<String>) {
        self.report(Progress::new(current, total, message));
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
