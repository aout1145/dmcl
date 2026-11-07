//! 任务图构图与运行入口。

use std::any::Any;
use std::error::Error;
use std::fmt;
use std::marker::PhantomData;
use std::sync::Arc;

use anyhow::Result;

use crate::bus::{EventBus, EventReceiver};
use crate::context::{CancelHandle, TaskContext};
use crate::deps::{Dependency, Deps, Handle, NodeSlot, StaticHandle, TaskHandle};
use crate::event::TaskEvent;
use crate::progress::ProgressHandle;
use crate::task::Task;

/// 一个只用于显示的步骤。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StepId(pub(crate) u32);

/// Step 的静态定义。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepDef {
    pub id: StepId,
    pub name: String,
    pub order: usize,
}

/// 构图期检查发现的错误。
#[derive(Debug)]
pub enum GraphError {
    /// sink 不属于这张图。
    UnknownSink { task: crate::event::TaskRef },
    /// sink 自己还有 dependent，不是最终节点。
    SinkHasDependents { task: crate::event::TaskRef },
    /// 除 sink 外还有别的 sink。
    MultipleSinks { tasks: Vec<crate::event::TaskRef> },
}

impl fmt::Display for GraphError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GraphError::UnknownSink { task } => {
                write!(f, "最终节点「{}」不属于这张图", task.name())
            }
            GraphError::SinkHasDependents { task } => {
                write!(f, "节点「{}」还有下游依赖，不能作为最终节点", task.name())
            }
            GraphError::MultipleSinks { tasks } => {
                let names: Vec<_> = tasks.iter().map(|task| task.name()).collect();
                write!(f, "除最终节点外还有别的终点节点：{}", names.join("、"))
            }
        }
    }
}

impl Error for GraphError {}

/// 类型擦除后的节点任务：取出依赖值、执行任务、把输出装箱。
#[async_trait::async_trait]
pub(crate) trait NodeTask: Send + Sync {
    async fn run(&self, ctx: TaskContext) -> Result<Arc<dyn Any + Send + Sync>>;
}

struct TypedTask<D, T, O> {
    deps: D,
    task: T,
    marker: PhantomData<fn() -> O>,
}

/// 把「依赖 + 任务」擦成 [`NodeTask`]。
pub(crate) fn node_task<D, T, O>(deps: D, task: T) -> Box<dyn NodeTask>
where
    D: Deps,
    T: Task<D::Output, O> + 'static,
    O: Send + Sync + 'static,
{
    Box::new(TypedTask {
        deps,
        task,
        marker: PhantomData,
    })
}

#[async_trait::async_trait]
impl<D, T, O> NodeTask for TypedTask<D, T, O>
where
    D: Deps,
    T: Task<D::Output, O>,
    O: Send + Sync + 'static,
{
    async fn run(&self, ctx: TaskContext) -> Result<Arc<dyn Any + Send + Sync>> {
        let input = self.deps.resolve();
        let output = self.task.run(input, ctx).await?;
        Ok(Arc::new(output))
    }
}

struct PendingNode {
    slot: Arc<NodeSlot>,
    deps: Vec<Dependency>,
    task: Box<dyn NodeTask>,
}

pub(crate) struct BuiltNode {
    pub(crate) slot: Arc<NodeSlot>,
    pub(crate) deps: Vec<usize>,
    pub(crate) task: Box<dyn NodeTask>,
}

/// 任务图的构图器。
pub struct TaskGraphBuilder {
    name: String,
    graph: u64,
    steps: Vec<StepDef>,
    nodes: Vec<PendingNode>,
    progress: ProgressHandle,
}

impl TaskGraphBuilder {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            graph: next_graph_id(),
            steps: Vec::new(),
            nodes: Vec::new(),
            progress: ProgressHandle::new(),
        }
    }

    /// 整张图共享的下载进度。
    ///
    /// 构图阶段就能拿到它，用来构造需要共享同一份进度的下载上下文。
    pub fn progress(&self) -> ProgressHandle {
        self.progress.clone()
    }

    /// 声明一个只用于显示的步骤；顺序由声明顺序决定。
    pub fn step(&mut self, name: impl Into<String>) -> StepId {
        let order = self.steps.len();
        let id = StepId(order as u32);
        self.steps.push(StepDef {
            id,
            name: name.into(),
            order,
        });
        id
    }

    /// 添加静态节点；`deps` 只能引用已经创建的静态节点。
    pub fn add<D, T, O>(
        &mut self,
        step: StepId,
        name: impl Into<String>,
        deps: D,
        task: T,
    ) -> Handle<O>
    where
        D: Deps,
        T: Task<D::Output, O> + 'static,
        O: Send + Sync + 'static,
    {
        assert!(
            (step.0 as usize) < self.steps.len(),
            "任务节点引用了不属于这张图的 StepId"
        );

        let id = self.nodes.len();
        let dependencies = deps.dependencies();
        for dependency in &dependencies {
            assert_eq!(
                dependency.graph, self.graph,
                "任务节点依赖了另一张图里的句柄"
            );
            assert!(dependency.node < id, "静态任务只能依赖已经创建的静态节点");
        }

        let slot = NodeSlot::new(self.graph, id, step, name.into().into());
        self.nodes.push(PendingNode {
            slot: slot.clone(),
            deps: dependencies,
            task: node_task(deps, task),
        });
        Arc::new(StaticHandle::new(slot))
    }

    /// 结束构图，检查唯一 sink，产出可运行的图。
    pub fn build<O>(self, sink: Handle<O>) -> Result<TaskGraph<O>, GraphError>
    where
        O: Send + Sync + 'static,
    {
        let sink_dependency = sink.slot_dependency();
        if sink_dependency.graph != self.graph || sink_dependency.node >= self.nodes.len() {
            return Err(GraphError::UnknownSink {
                task: sink.task_ref(),
            });
        }
        let sink_id = sink_dependency.node;

        let mut dependents = vec![Vec::new(); self.nodes.len()];
        for (index, node) in self.nodes.iter().enumerate() {
            for dependency in &node.deps {
                dependents[dependency.node].push(index);
            }
        }

        if !dependents[sink_id].is_empty() {
            return Err(GraphError::SinkHasDependents {
                task: self.nodes[sink_id].slot.task_ref(),
            });
        }

        let mut other_sinks = Vec::new();
        for (index, node) in self.nodes.iter().enumerate() {
            if index != sink_id && dependents[index].is_empty() {
                other_sinks.push(node.slot.task_ref());
            }
        }
        if !other_sinks.is_empty() {
            return Err(GraphError::MultipleSinks { tasks: other_sinks });
        }

        let nodes = self
            .nodes
            .into_iter()
            .map(|node| BuiltNode {
                slot: node.slot,
                deps: node.deps.into_iter().map(|dep| dep.node).collect(),
                task: node.task,
            })
            .collect();

        Ok(TaskGraph {
            name: self.name,
            graph: self.graph,
            steps: self.steps,
            nodes,
            dependents,
            sink: sink_id,
            bus: EventBus::new(),
            cancel: CancelHandle::default(),
            progress: self.progress,
            marker: PhantomData,
        })
    }
}

/// 已通过 `build` 校验、可以运行的任务图。
pub struct TaskGraph<O> {
    pub(crate) name: String,
    pub(crate) graph: u64,
    pub(crate) steps: Vec<StepDef>,
    pub(crate) nodes: Vec<BuiltNode>,
    pub(crate) dependents: Vec<Vec<usize>>,
    pub(crate) sink: usize,
    pub(crate) bus: Arc<EventBus>,
    pub(crate) cancel: CancelHandle,
    pub(crate) progress: ProgressHandle,
    pub(crate) marker: PhantomData<fn() -> O>,
}

impl<O: Send + Sync + 'static> TaskGraph<O> {
    /// 订阅事件流；只收到订阅之后产生的事件。
    pub fn subscribe(&self) -> EventReceiver {
        self.bus.subscribe()
    }

    /// 取消句柄，可在运行期间从任意线程触发取消。
    pub fn cancel_handle(&self) -> CancelHandle {
        self.cancel.clone()
    }

    /// 整张图共享的下载进度。
    pub fn progress(&self) -> ProgressHandle {
        self.progress.clone()
    }

    /// 构图期声明的全部 Step（按声明顺序）。
    pub fn steps(&self) -> &[StepDef] {
        &self.steps
    }

    /// 运行整张图，边跑边把事件交给 `on_event`，返回唯一最终节点的输出。
    ///
    /// 不关心事件时传 `|_| {}`。
    pub async fn run<F>(self, mut on_event: F) -> Result<Arc<O>>
    where
        F: FnMut(TaskEvent),
    {
        let bus = self.bus.clone();
        let mut receiver = bus.subscribe();
        let running = self.execute();
        tokio::pin!(running);

        loop {
            tokio::select! {
                biased;
                result = &mut running => {
                    while let Some(event) = receiver.try_recv() {
                        on_event(event);
                    }
                    let output = result?;
                    return Ok(output
                        .downcast::<O>()
                        .expect("任务图的最终输出类型没有对齐"));
                }
                event = receiver.recv(), if !bus.is_closed() => {
                    if let Some(event) = event {
                        on_event(event);
                    }
                }
            }
        }
    }
}

/// 取一个与其它图区分的标记，用于发现跨图句柄。
fn next_graph_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

// 供 `build` 使用：句柄的依赖信息。
trait SinkDependency {
    fn slot_dependency(&self) -> Dependency;
}

impl<O: Send + Sync + 'static> SinkDependency for Arc<dyn TaskHandle<O>> {
    fn slot_dependency(&self) -> Dependency {
        crate::deps::sealed::SealedHandle::dependency(self.as_ref())
    }
}
