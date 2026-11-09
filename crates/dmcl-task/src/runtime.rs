//! 任务图调度器：FailFast、取消、动态节点与回滚。

use std::any::Any;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use anyhow::Result;
use tokio::sync::{mpsc, Notify};
use tokio::task::JoinSet;
use tokio::time::{interval, MissedTickBehavior};

use crate::bus::EventBus;
use crate::context::{AddTaskError, Cancelled, Cleanup, TaskContext};
use crate::deps::{Deps, DynamicHandle, DynamicHandleImpl, NodeSlot};
use crate::event::{TaskEvent, TaskStatus};
use crate::graph::{node_task, NodeTask, StepDef, StepId, TaskGraph};
use crate::progress::ProgressHandle;
use crate::task::Task;

struct CleanupList {
    active: bool,
    items: Vec<Arc<dyn Cleanup>>,
}

struct NodeState {
    slot: Arc<NodeSlot>,
    deps: Vec<usize>,
    task: Option<Box<dyn NodeTask>>,
    dependents: Vec<usize>,
    pending_deps: usize,
    status: TaskStatus,
    body_done: bool,
    child_failed: bool,
    dynamic_pending: usize,
    creator: Option<usize>,
    cleanups: Mutex<CleanupList>,
    /// 节点最终状态确定时唤醒 `DynamicHandle::wait`。
    done: Arc<Notify>,
    /// 动态子节点数量变化时唤醒创建者。
    children_done: Arc<Notify>,
    error: Option<String>,
}

struct RuntimeInner {
    nodes: Vec<NodeState>,
    remaining: usize,
    first_error: Option<anyhow::Error>,
    failed: bool,
    closed: bool,
    completed_cleanups: Vec<(usize, Arc<dyn Cleanup>)>,
}

pub(crate) struct Runtime {
    graph: u64,
    name: String,
    steps: Vec<StepDef>,
    pub(crate) bus: Arc<EventBus>,
    pub(crate) cancel: crate::context::CancelHandle,
    pub(crate) progress: ProgressHandle,
    sink: usize,
    inner: Mutex<RuntimeInner>,
    ready: Mutex<VecDeque<usize>>,
    ready_notify: Notify,
}

struct Finished {
    node: usize,
    result: Result<()>,
}

impl Runtime {
    fn new<O>(graph: TaskGraph<O>) -> Arc<Self> {
        let TaskGraph {
            name,
            graph: graph_id,
            steps,
            nodes,
            dependents,
            sink,
            bus,
            cancel,
            progress,
            marker: _,
        } = graph;

        let mut states = Vec::with_capacity(nodes.len());
        for (index, node) in nodes.into_iter().enumerate() {
            states.push(NodeState {
                slot: node.slot,
                deps: node.deps,
                task: Some(node.task),
                dependents: dependents[index].clone(),
                pending_deps: 0,
                status: TaskStatus::Waiting,
                body_done: false,
                child_failed: false,
                dynamic_pending: 0,
                creator: None,
                cleanups: Mutex::new(CleanupList {
                    active: true,
                    items: Vec::new(),
                }),
                done: Arc::new(Notify::new()),
                children_done: Arc::new(Notify::new()),
                error: None,
            });
        }
        let remaining = states.len();
        for state in &mut states {
            state.pending_deps = state.deps.len();
        }

        let runtime = Arc::new(Self {
            graph: graph_id,
            name,
            steps,
            bus,
            cancel,
            progress,
            sink,
            inner: Mutex::new(RuntimeInner {
                nodes: states,
                remaining,
                first_error: None,
                failed: false,
                closed: false,
                completed_cleanups: Vec::new(),
            }),
            ready: Mutex::new(VecDeque::new()),
            ready_notify: Notify::new(),
        });

        let initial: Vec<usize> = {
            let inner = runtime.lock_inner();
            inner
                .nodes
                .iter()
                .enumerate()
                .filter(|(_, node)| node.pending_deps == 0)
                .map(|(index, _)| index)
                .collect()
        };
        runtime.lock_ready().extend(initial);
        runtime
    }

    fn lock_inner(&self) -> MutexGuard<'_, RuntimeInner> {
        self.inner.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn lock_ready(&self) -> MutexGuard<'_, VecDeque<usize>> {
        self.ready.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn enqueue_ready(&self, node: usize) {
        self.lock_ready().push_back(node);
        self.ready_notify.notify_one();
    }

    fn publish_initial(&self) {
        self.bus.publish(TaskEvent::GraphStarted {
            name: self.name.clone(),
            steps: self.steps.clone(),
        });
        let nodes = self.lock_inner();
        for node in &nodes.nodes {
            self.bus.publish(TaskEvent::TaskAdded {
                task: node.slot.task_ref(),
            });
        }
    }

    fn remaining(&self) -> usize {
        self.lock_inner().remaining
    }

    fn is_failed(&self) -> bool {
        self.lock_inner().failed
    }

    fn publish_progress(&self) {
        let snapshot = self.progress.snapshot();
        self.bus.publish(TaskEvent::Progress {
            downloaded: snapshot.downloaded,
            total: snapshot.total,
        });
    }

    fn start_ready(
        self: &Arc<Self>,
        sender: &mpsc::UnboundedSender<Finished>,
        joins: &mut JoinSet<()>,
    ) {
        loop {
            let Some(node) = self.lock_ready().pop_front() else {
                break;
            };

            let running = {
                let mut inner = self.lock_inner();
                if inner.closed {
                    continue;
                }
                let state = &mut inner.nodes[node];
                if state.status != TaskStatus::Waiting || state.pending_deps != 0 {
                    continue;
                }
                let Some(task) = state.task.take() else {
                    continue;
                };
                state.status = TaskStatus::Running;
                let slot = state.slot.clone();
                slot.set_status(TaskStatus::Running);
                let ctx = TaskContext::graph(self.clone(), node, slot.clone());
                (task, ctx, slot)
            };

            let (task, ctx, slot) = running;
            self.bus.publish(TaskEvent::TaskStarted {
                task: slot.task_ref(),
            });
            let runtime = self.clone();
            let sender = sender.clone();
            joins.spawn(async move {
                run_node(runtime, node, task, ctx, sender).await;
            });
        }
    }

    fn handle_finished(&self, message: Finished) {
        let mut new_ready = Vec::new();
        let mut failed_now = false;

        {
            let mut inner = self.lock_inner();
            match message.result {
                Ok(()) => {
                    let (dependents, creator, slot) = {
                        let state = &mut inner.nodes[message.node];
                        state.status = TaskStatus::Finished;
                        state.body_done = true;
                        state.slot.set_status(TaskStatus::Finished);
                        state.done.notify_one();
                        (state.dependents.clone(), state.creator, state.slot.clone())
                    };
                    inner.remaining -= 1;
                    self.bus.publish(TaskEvent::TaskFinished {
                        task: slot.task_ref(),
                    });

                    if let Some(creator) = creator {
                        let creator = &mut inner.nodes[creator];
                        creator.dynamic_pending = creator.dynamic_pending.saturating_sub(1);
                        creator.children_done.notify_one();
                    }
                    for dependent in dependents {
                        let state = &mut inner.nodes[dependent];
                        state.pending_deps = state.pending_deps.saturating_sub(1);
                        if state.pending_deps == 0 {
                            new_ready.push(dependent);
                        }
                    }
                }
                Err(err) => {
                    let (slot, creator) = {
                        let state = &mut inner.nodes[message.node];
                        state.status = TaskStatus::Failed;
                        state.body_done = true;
                        state.slot.set_status(TaskStatus::Failed);
                        state.error = Some(format!("{err:#}"));
                        state.done.notify_one();
                        (state.slot.clone(), state.creator)
                    };
                    inner.remaining -= 1;

                    if let Some(creator) = creator {
                        let creator = &mut inner.nodes[creator];
                        creator.dynamic_pending = creator.dynamic_pending.saturating_sub(1);
                        creator.child_failed = true;
                        creator.children_done.notify_one();
                    }

                    self.bus.publish(TaskEvent::TaskFailed {
                        task: slot.task_ref(),
                    });
                    if !inner.failed {
                        inner.failed = true;
                        inner.closed = true;
                        inner.first_error = Some(err.context(format!("任务「{}」失败", slot.name)));
                        failed_now = true;
                    }
                }
            }
        }

        for node in new_ready {
            self.enqueue_ready(node);
        }
        if failed_now {
            self.cancel.cancel();
            self.skip_waiting();
        }
    }

    /// 把尚未开始的节点标为跳过，并通知它们的创建者。
    fn skip_waiting(&self) {
        let mut skipped = Vec::new();
        {
            let mut inner = self.lock_inner();
            for node in 0..inner.nodes.len() {
                if inner.nodes[node].status != TaskStatus::Waiting {
                    continue;
                }
                let (slot, creator) = {
                    let state = &mut inner.nodes[node];
                    state.status = TaskStatus::Skipped;
                    state.body_done = true;
                    state.slot.set_status(TaskStatus::Skipped);
                    state.done.notify_one();
                    (state.slot.clone(), state.creator)
                };
                inner.remaining -= 1;

                if let Some(creator) = creator {
                    let creator = &mut inner.nodes[creator];
                    creator.dynamic_pending = creator.dynamic_pending.saturating_sub(1);
                    creator.child_failed = true;
                    creator.children_done.notify_one();
                }
                skipped.push(slot);
            }
        }
        for slot in skipped {
            self.bus.publish(TaskEvent::TaskSkipped {
                task: slot.task_ref(),
            });
        }
    }

    fn fail(&self, error: anyhow::Error) {
        {
            let mut inner = self.lock_inner();
            if inner.failed {
                return;
            }
            inner.failed = true;
            inner.closed = true;
            inner.first_error = Some(error);
        }
        self.cancel.cancel();
        self.skip_waiting();
    }

    pub(crate) fn register_cleanup(&self, node: usize, cleanup: Arc<dyn Cleanup>) -> bool {
        let inner = self.lock_inner();
        let mut cleanups = inner.nodes[node]
            .cleanups
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !cleanups.active {
            return false;
        }
        cleanups.items.push(cleanup);
        true
    }

    async fn run_node_cleanups(&self, node: usize, ctx: TaskContext) {
        let cleanups = {
            let inner = self.lock_inner();
            let mut cleanups = inner.nodes[node]
                .cleanups
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            cleanups.active = false;
            std::mem::take(&mut cleanups.items)
        };
        for cleanup in cleanups.into_iter().rev() {
            cleanup.run(ctx.clone()).await;
        }
    }

    fn complete_success(&self, node: usize, output: Arc<dyn Any + Send + Sync>) {
        let (slot, cleanups) = {
            let mut inner = self.lock_inner();
            let state = &mut inner.nodes[node];
            let mut list = state
                .cleanups
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            list.active = false;
            let cleanups = std::mem::take(&mut list.items);
            (state.slot.clone(), cleanups)
        };
        slot.set_output(output);
        let mut inner = self.lock_inner();
        for cleanup in cleanups {
            inner.completed_cleanups.push((node, cleanup));
        }
    }

    async fn wait_children(&self, node: usize) -> Result<()> {
        loop {
            let notify = {
                let inner = self.lock_inner();
                if inner.failed {
                    return Err(anyhow::Error::new(Cancelled));
                }
                let state = &inner.nodes[node];
                if state.child_failed {
                    return Err(anyhow::anyhow!("动态子任务失败"));
                }
                if state.dynamic_pending == 0 {
                    return Ok(());
                }
                state.children_done.clone()
            };
            notify.notified().await;
        }
    }

    pub(crate) async fn wait_for_node(&self, node: usize) -> Result<()> {
        loop {
            let (notify, status, error) = {
                let inner = self.lock_inner();
                let state = &inner.nodes[node];
                (state.done.clone(), state.status, state.error.clone())
            };
            match status {
                TaskStatus::Finished => return Ok(()),
                TaskStatus::Failed => {
                    return Err(anyhow::anyhow!(
                        "动态任务失败: {}",
                        error.unwrap_or_else(|| "未知错误".to_string())
                    ));
                }
                TaskStatus::Skipped => {
                    return Err(anyhow::anyhow!("动态任务被跳过"));
                }
                TaskStatus::Waiting | TaskStatus::Running => {
                    notify.notified().await;
                }
            }
        }
    }

    pub(crate) fn add_task<D, T, O>(
        self: &Arc<Self>,
        creator: usize,
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
        assert!(
            (step.0 as usize) < self.steps.len(),
            "动态任务引用了不属于这张图的 StepId"
        );
        let dependencies = deps.dependencies();
        let name: Arc<str> = name.into().into();

        let mut inner = self.lock_inner();
        if inner.closed {
            return Err(AddTaskError::GraphClosed);
        }
        {
            let creator_state = &inner.nodes[creator];
            if creator_state.body_done || creator_state.status != TaskStatus::Running {
                return Err(AddTaskError::GraphClosed);
            }
        }

        for dependency in &dependencies {
            assert_eq!(
                dependency.graph, self.graph,
                "动态任务依赖了另一张图里的句柄"
            );
            if dependency.node >= inner.nodes.len() {
                panic!("动态任务依赖了不存在的节点");
            }
            let dependency_state = &inner.nodes[dependency.node];
            if dependency_state.creator.is_some() {
                return Err(AddTaskError::DynamicDependencyNotAllowed {
                    task: dependency_state.slot.task_ref(),
                });
            }
            if reaches(&inner, dependency.node, creator) {
                let slot = NodeSlot::new(self.graph, inner.nodes.len(), step, name);
                return Err(AddTaskError::Cycle {
                    task: slot.task_ref(),
                });
            }
        }

        let id = inner.nodes.len();
        let slot = NodeSlot::new(self.graph, id, step, name);

        let mut pending_deps = 0;
        for dependency in &dependencies {
            let dep = &inner.nodes[dependency.node];
            if dep.status == TaskStatus::Finished {
                continue;
            }
            pending_deps += 1;
        }
        // 先建立依赖边，再插入节点，避免在持有 `nodes` 元素引用时再次借用。
        let dependency_ids: Vec<usize> = dependencies.iter().map(|dep| dep.node).collect();
        for dependency in &dependency_ids {
            let dep = &mut inner.nodes[*dependency];
            if dep.status != TaskStatus::Finished {
                dep.dependents.push(id);
            }
        }

        inner.nodes[creator].dynamic_pending += 1;
        inner.nodes.push(NodeState {
            slot: slot.clone(),
            deps: dependency_ids,
            task: Some(node_task(deps, task)),
            dependents: Vec::new(),
            pending_deps,
            status: TaskStatus::Waiting,
            body_done: false,
            child_failed: false,
            dynamic_pending: 0,
            creator: Some(creator),
            cleanups: Mutex::new(CleanupList {
                active: true,
                items: Vec::new(),
            }),
            done: Arc::new(Notify::new()),
            children_done: Arc::new(Notify::new()),
            error: None,
        });
        inner.remaining += 1;
        drop(inner);

        self.bus.publish(TaskEvent::TaskAdded {
            task: slot.task_ref(),
        });
        if pending_deps == 0 {
            self.enqueue_ready(id);
        }

        Ok(Arc::new(DynamicHandleImpl::new(slot, self.clone())))
    }

    async fn finish(self: &Arc<Self>) -> Result<Arc<dyn Any + Send + Sync>> {
        let first_error = self.lock_inner().first_error.take();

        if let Some(error) = first_error {
            let cleanups = {
                let mut inner = self.lock_inner();
                std::mem::take(&mut inner.completed_cleanups)
            };
            for (node, cleanup) in cleanups.into_iter().rev() {
                let slot = self.lock_inner().nodes[node].slot.clone();
                let ctx = TaskContext::graph(self.clone(), node, slot);
                cleanup.run(ctx).await;
            }
            self.publish_progress();
            self.bus.publish(TaskEvent::GraphFailed);
            self.bus.close();
            return Err(error);
        }

        let output = self.lock_inner().nodes[self.sink].slot.output_any();
        let output = output.expect("成功的任务图缺少最终节点输出");
        self.publish_progress();
        self.bus.publish(TaskEvent::GraphFinished);
        self.bus.close();
        Ok(output)
    }
}

/// 沿依赖边检查 `from` 是否能到达 `target`。
fn reaches(inner: &RuntimeInner, from: usize, target: usize) -> bool {
    let mut stack = vec![from];
    let mut seen = vec![false; inner.nodes.len()];
    while let Some(node) = stack.pop() {
        if node == target {
            return true;
        }
        if seen[node] {
            continue;
        }
        seen[node] = true;
        stack.extend(inner.nodes[node].deps.iter().copied());
    }
    false
}

async fn run_node(
    runtime: Arc<Runtime>,
    node: usize,
    task: Box<dyn NodeTask>,
    ctx: TaskContext,
    sender: mpsc::UnboundedSender<Finished>,
) {
    let result = match task.run(ctx.clone()).await {
        Ok(output) => match runtime.wait_children(node).await {
            Ok(()) => {
                runtime.complete_success(node, output);
                Ok(())
            }
            Err(error) => Err(error),
        },
        Err(error) => Err(error),
    };

    let result = match result {
        Ok(()) => Ok(()),
        Err(error) => {
            runtime.run_node_cleanups(node, ctx).await;
            Err(error)
        }
    };
    let _ = sender.send(Finished { node, result });
}

impl<O: Send + Sync + 'static> TaskGraph<O> {
    pub(crate) async fn execute(self) -> Result<Arc<dyn Any + Send + Sync>> {
        let runtime = Runtime::new(self);
        runtime.publish_initial();

        let (sender, mut receiver) = mpsc::unbounded_channel::<Finished>();
        let mut joins = JoinSet::new();
        let mut ticker = interval(Duration::from_millis(100));
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

        loop {
            runtime.start_ready(&sender, &mut joins);
            if runtime.remaining() == 0 {
                break;
            }
            tokio::select! {
                biased;
                Some(message) = receiver.recv() => runtime.handle_finished(message),
                _ = runtime.ready_notify.notified(), if !runtime.is_failed() => {}
                _ = runtime.cancel.cancelled(), if !runtime.is_failed() => {
                    runtime.fail(anyhow::Error::new(Cancelled));
                }
                _ = ticker.tick() => runtime.publish_progress(),
            }
        }

        while joins.join_next().await.is_some() {}
        runtime.finish().await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use crate::context::AddTaskError;
    use crate::event::TaskEvent;
    use crate::graph::{GraphError, TaskGraph, TaskGraphBuilder};
    use crate::task::task_fn;
    use crate::Handle;

    async fn collect<O: Send + Sync + 'static>(
        graph: TaskGraph<O>,
    ) -> (anyhow::Result<Arc<O>>, Vec<TaskEvent>) {
        let events = Arc::new(Mutex::new(Vec::new()));
        let collected = events.clone();
        let result = graph
            .run(move |event| collected.lock().unwrap().push(event))
            .await;
        let events = events.lock().unwrap().clone();
        (result, events)
    }

    #[tokio::test]
    async fn dependencies_inject_single_and_multiple_values() {
        let mut builder = TaskGraphBuilder::new("依赖注入");
        let step = builder.step("步骤");

        let first = builder.add(
            step,
            "一",
            (),
            task_fn(|_: (), _ctx| async move { Ok(1u32) }),
        );
        let second = builder.add(
            step,
            "二",
            first.clone(),
            task_fn(|value: Arc<u32>, _ctx| async move { Ok(*value + 1) }),
        );
        let third = builder.add(
            step,
            "三",
            (first, second),
            task_fn(|(a, b): (Arc<u32>, Arc<u32>), _ctx| async move { Ok(*a + *b) }),
        );
        let sink = builder.add(
            step,
            "汇总",
            third,
            task_fn(|value: Arc<u32>, _ctx| async move { Ok(*value) }),
        );

        let graph = builder.build(sink).unwrap();
        let (result, _) = collect(graph).await;
        assert_eq!(*result.unwrap(), 3);
    }

    #[tokio::test]
    async fn a_vec_dependency_collects_a_variable_number_of_values() {
        let mut builder = TaskGraphBuilder::new("动态数量依赖");
        let step = builder.step("步骤");

        let scalar = builder.add(
            step,
            "固定依赖",
            (),
            task_fn(|_: (), _ctx| async move { Ok(10u32) }),
        );
        let values: Vec<Handle<u32>> = (0..3)
            .map(|value| {
                builder.add(
                    step,
                    format!("值 {value}"),
                    (),
                    task_fn(move |_: (), _ctx| async move { Ok(value) }),
                )
            })
            .collect();
        let sink = builder.add(
            step,
            "汇总",
            (scalar, values),
            task_fn(
                |(scalar, values): (Arc<u32>, Vec<Arc<u32>>), _ctx| async move {
                    Ok(*scalar + values.iter().map(|value| **value).sum::<u32>())
                },
            ),
        );

        let graph = builder.build(sink).unwrap();
        let (result, _) = collect(graph).await;
        assert_eq!(*result.unwrap(), 13);
    }

    #[tokio::test]
    async fn an_unused_dependency_can_be_ignored() {
        let mut builder = TaskGraphBuilder::new("忽略依赖");
        let step = builder.step("步骤");

        let value = builder.add(
            step,
            "值",
            (),
            task_fn(|_: (), _ctx| async move { Ok(9u32) }),
        );
        let sink = builder.add(
            step,
            "忽略",
            value,
            task_fn(|_value: Arc<u32>, _ctx| async move { Ok(()) }),
        );

        let graph = builder.build(sink).unwrap();
        let (result, _) = collect(graph).await;
        assert_eq!(*result.unwrap(), ());
    }

    #[tokio::test]
    async fn a_final_node_can_return_unit() {
        let mut builder = TaskGraphBuilder::new("单元输出");
        let step = builder.step("步骤");
        let sink = builder.add(
            step,
            "真实最后一步",
            (),
            task_fn(|_: (), _ctx| async move { Ok(()) }),
        );
        let graph = builder.build(sink).unwrap();
        let (result, _) = collect(graph).await;
        assert_eq!(*result.unwrap(), ());
    }

    #[tokio::test]
    #[should_panic(expected = "另一张图里的句柄")]
    async fn a_dependency_from_another_graph_panics_during_building() {
        let mut first = TaskGraphBuilder::new("一");
        let first_step = first.step("步骤");

        let mut second = TaskGraphBuilder::new("二");
        let second_step = second.step("步骤");
        let foreign = second.add(
            second_step,
            "外部",
            (),
            task_fn(|_: (), _ctx| async move { Ok(()) }),
        );

        let _ = first.add(
            first_step,
            "使用外部依赖",
            foreign,
            task_fn(|_value: Arc<()>, _ctx| async move { Ok(()) }),
        );
    }

    #[tokio::test]
    async fn build_rejects_a_sink_from_another_graph() {
        let mut first = TaskGraphBuilder::new("一");
        let step = first.step("步骤");
        let _ = first.add(step, "a", (), task_fn(|_: (), _ctx| async move { Ok(()) }));

        let mut second = TaskGraphBuilder::new("二");
        let step = second.step("步骤");
        let foreign = second.add(step, "b", (), task_fn(|_: (), _ctx| async move { Ok(()) }));

        assert!(matches!(
            first.build(foreign),
            Err(GraphError::UnknownSink { .. })
        ));
    }

    #[tokio::test]
    async fn build_rejects_a_sink_with_dependents() {
        let mut builder = TaskGraphBuilder::new("有下游");
        let step = builder.step("步骤");
        let root = builder.add(
            step,
            "root",
            (),
            task_fn(|_: (), _ctx| async move { Ok(()) }),
        );
        let _sink = builder.add(
            step,
            "sink",
            root.clone(),
            task_fn(|_value: Arc<()>, _ctx| async move { Ok(()) }),
        );

        assert!(matches!(
            builder.build(root),
            Err(GraphError::SinkHasDependents { .. })
        ));
    }

    #[tokio::test]
    async fn build_rejects_other_sinks() {
        let mut builder = TaskGraphBuilder::new("多终点");
        let step = builder.step("步骤");
        let sink = builder.add(step, "a", (), task_fn(|_: (), _ctx| async move { Ok(()) }));
        let _other = builder.add(step, "b", (), task_fn(|_: (), _ctx| async move { Ok(()) }));

        match builder.build(sink) {
            Err(GraphError::MultipleSinks { tasks }) => assert_eq!(tasks.len(), 1),
            Err(error) => panic!("应当是 MultipleSinks，实际是 {error:?}"),
            Ok(_) => panic!("应当拒绝多余的终点"),
        }
    }

    #[tokio::test]
    async fn dynamic_children_are_joined_without_waiting() {
        let mut builder = TaskGraphBuilder::new("展开");
        let step = builder.step("下载");
        let finished = Arc::new(AtomicUsize::new(0));
        let root_done = Arc::new(AtomicBool::new(false));

        let finished_for_root = finished.clone();
        let root_done_for_root = root_done.clone();
        let root = builder.add(
            step,
            "准备",
            (),
            task_fn(move |_: (), ctx| {
                let finished = finished_for_root.clone();
                let root_done = root_done_for_root.clone();
                async move {
                    for index in 0..8 {
                        let finished = finished.clone();
                        ctx.add_task(
                            step,
                            format!("文件 {index}"),
                            (),
                            task_fn(move |_: (), _ctx| {
                                let finished = finished.clone();
                                async move {
                                    tokio::time::sleep(Duration::from_millis(5)).await;
                                    finished.fetch_add(1, Ordering::SeqCst);
                                    Ok(())
                                }
                            }),
                        )?;
                    }
                    root_done.store(true, Ordering::SeqCst);
                    Ok(())
                }
            }),
        );
        let sink = builder.add(
            step,
            "结束",
            root,
            task_fn(|_value: Arc<()>, _ctx| async move { Ok(()) }),
        );

        let graph = builder.build(sink).unwrap();
        let (result, events) = collect(graph).await;
        result.unwrap();
        assert!(root_done.load(Ordering::SeqCst));
        assert_eq!(finished.load(Ordering::SeqCst), 8);

        // 创建者的完成事件必须晚于所有动态子节点。
        let root_finished = events
            .iter()
            .position(
                |event| matches!(event, TaskEvent::TaskFinished { task } if task.name() == "准备"),
            )
            .unwrap();
        assert!(events[..root_finished].iter().any(|event| matches!(
            event,
            TaskEvent::TaskFinished { task } if task.name().starts_with("文件 ")
        )));
    }

    #[tokio::test]
    async fn wait_returns_the_dynamic_output() {
        let mut builder = TaskGraphBuilder::new("等待动态输出");
        let step = builder.step("步骤");
        let root = builder.add(
            step,
            "创建者",
            (),
            task_fn(move |_: (), ctx| async move {
                let handle = ctx.add_task(
                    step,
                    "动态值",
                    (),
                    task_fn(|_: (), _ctx| async move { Ok(7u32) }),
                )?;
                let value = handle.wait().await?;
                Ok(*value)
            }),
        );
        let sink = builder.add(
            step,
            "结束",
            root,
            task_fn(|value: Arc<u32>, _ctx| async move { Ok(*value) }),
        );
        let graph = builder.build(sink).unwrap();
        let (result, _) = collect(graph).await;
        assert_eq!(*result.unwrap(), 7);
    }

    #[tokio::test]
    async fn a_dynamic_node_can_depend_on_a_static_node() {
        let mut builder = TaskGraphBuilder::new("动态依赖静态");
        let step = builder.step("步骤");
        let base = builder.add(
            step,
            "静态",
            (),
            task_fn(|_: (), _ctx| async move { Ok(5u32) }),
        );

        let base_for_root = base.clone();
        let root = builder.add(
            step,
            "创建者",
            base.clone(),
            task_fn(move |_value: Arc<u32>, ctx| {
                let base = base_for_root.clone();
                async move {
                    let handle = ctx.add_task(
                        step,
                        "动态",
                        base,
                        task_fn(|value: Arc<u32>, _ctx| async move { Ok(*value + 1) }),
                    )?;
                    let value = handle.wait().await?;
                    Ok(*value)
                }
            }),
        );
        let sink = builder.add(
            step,
            "结束",
            root,
            task_fn(|value: Arc<u32>, _ctx| async move { Ok(*value) }),
        );
        let graph = builder.build(sink).unwrap();
        let (result, _) = collect(graph).await;
        assert_eq!(*result.unwrap(), 6);
    }

    #[tokio::test]
    async fn a_dynamic_handle_upcast_into_deps_is_rejected_at_runtime() {
        let mut builder = TaskGraphBuilder::new("动态依赖非法");
        let step = builder.step("步骤");
        let root = builder.add(
            step,
            "创建者",
            (),
            task_fn(move |_: (), ctx| async move {
                let dynamic = ctx.add_task(
                    step,
                    "动态",
                    (),
                    task_fn(|_: (), _ctx| async move { Ok(1u32) }),
                )?;
                let upcast: Handle<u32> = dynamic.clone();
                match ctx.add_task(
                    step,
                    "非法",
                    upcast,
                    task_fn(|_value: Arc<u32>, _ctx| async move { Ok(()) }),
                ) {
                    Err(AddTaskError::DynamicDependencyNotAllowed { .. }) => {}
                    Err(other) => panic!("应当拒绝动态依赖，实际是 {other:?}"),
                    Ok(_) => panic!("应当拒绝动态依赖"),
                }
                let _ = dynamic.wait().await?;
                Ok(())
            }),
        );
        let sink = builder.add(
            step,
            "结束",
            root,
            task_fn(|_value: Arc<()>, _ctx| async move { Ok(()) }),
        );
        let graph = builder.build(sink).unwrap();
        let (result, _) = collect(graph).await;
        result.unwrap();
    }

    #[tokio::test]
    async fn a_dynamic_dependency_cycle_is_rejected() {
        let mut builder = TaskGraphBuilder::new("环");
        let step = builder.step("步骤");
        let downstream_slot: Arc<Mutex<Option<Handle<()>>>> = Arc::new(Mutex::new(None));

        let slot = downstream_slot.clone();
        let creator = builder.add(
            step,
            "创建者",
            (),
            task_fn(move |_: (), ctx| {
                let slot = slot.clone();
                async move {
                    let downstream = slot.lock().unwrap().clone().unwrap();
                    match ctx.add_task(
                        step,
                        "会成环的动态节点",
                        downstream,
                        task_fn(|_value: Arc<()>, _ctx| async move { Ok(()) }),
                    ) {
                        Err(AddTaskError::Cycle { .. }) => Ok(()),
                        Err(other) => panic!("应当检测到环，实际是 {other:?}"),
                        Ok(_) => panic!("应当检测到环"),
                    }
                }
            }),
        );
        let downstream = builder.add(
            step,
            "下游",
            creator.clone(),
            task_fn(|_value: Arc<()>, _ctx| async move { Ok(()) }),
        );
        *downstream_slot.lock().unwrap() = Some(downstream.clone());

        let graph = builder.build(downstream).unwrap();
        let (result, _) = collect(graph).await;
        result.unwrap();
    }

    #[tokio::test]
    async fn fail_fast_cancels_running_nodes_and_skips_waiting_ones() {
        let mut builder = TaskGraphBuilder::new("FailFast");
        let step = builder.step("步骤");

        let slow_cleaned = Arc::new(AtomicBool::new(false));
        let slow_cleaned_for_task = slow_cleaned.clone();
        let slow_started = Arc::new(AtomicBool::new(false));
        let slow_started_for_task = slow_started.clone();

        let failing = builder.add(
            step,
            "会失败",
            (),
            task_fn(|_: (), _ctx| async move {
                tokio::time::sleep(Duration::from_millis(30)).await;
                Err::<(), _>(anyhow::anyhow!("故意失败"))
            }),
        );
        let slow = builder.add(
            step,
            "运行中",
            (),
            task_fn(move |_: (), ctx| {
                let started = slow_started_for_task.clone();
                let cleaned = slow_cleaned_for_task.clone();
                async move {
                    started.store(true, Ordering::SeqCst);
                    ctx.on_rollback(move |_ctx| {
                        let cleaned = cleaned.clone();
                        async move {
                            cleaned.store(true, Ordering::SeqCst);
                        }
                    });
                    ctx.cancel_handle().cancelled().await;
                    ctx.check_cancelled()
                }
            }),
        );
        let after = builder.add(
            step,
            "等待中",
            failing.clone(),
            task_fn(|_value: Arc<()>, _ctx| async move { Ok(()) }),
        );
        let sink = builder.add(
            step,
            "结束",
            (slow, after),
            task_fn(|_: (Arc<()>, Arc<()>), _ctx| async move { Ok(()) }),
        );

        let graph = builder.build(sink).unwrap();
        let (result, events) = collect(graph).await;
        let error = result.unwrap_err();
        assert!(format!("{error:#}").contains("会失败"), "{error:#}");
        assert!(slow_started.load(Ordering::SeqCst));
        assert!(
            slow_cleaned.load(Ordering::SeqCst),
            "失败时应执行节点自身的回滚"
        );

        let status = |name: &str| {
            events.iter().find_map(|event| match event {
                TaskEvent::TaskFailed { task } if task.name() == name => Some("failed"),
                TaskEvent::TaskSkipped { task } if task.name() == name => Some("skipped"),
                TaskEvent::TaskFinished { task } if task.name() == name => Some("finished"),
                _ => None,
            })
        };
        assert_eq!(status("运行中"), Some("failed"));
        assert_eq!(status("等待中"), Some("skipped"));
        assert_eq!(status("结束"), Some("skipped"));
        assert!(matches!(events.last(), Some(TaskEvent::GraphFailed)));
    }

    #[tokio::test]
    async fn node_cleanups_run_in_reverse_registration_order() {
        let order = Arc::new(Mutex::new(Vec::new()));
        let mut builder = TaskGraphBuilder::new("回滚顺序");
        let step = builder.step("步骤");

        let first = order.clone();
        let second = order.clone();
        let sink = builder.add(
            step,
            "失败节点",
            (),
            task_fn(move |_: (), ctx| {
                let first = first.clone();
                let second = second.clone();
                async move {
                    ctx.on_rollback(move |_ctx| {
                        let first = first.clone();
                        async move {
                            first.lock().unwrap().push("先注册");
                        }
                    });
                    ctx.on_rollback(move |_ctx| {
                        let second = second.clone();
                        async move {
                            second.lock().unwrap().push("后注册");
                        }
                    });
                    Err::<(), _>(anyhow::anyhow!("失败"))
                }
            }),
        );

        let graph = builder.build(sink).unwrap();
        let (result, _) = collect(graph).await;
        result.unwrap_err();
        assert_eq!(*order.lock().unwrap(), vec!["后注册", "先注册"]);
    }

    #[tokio::test]
    async fn successful_node_cleanups_run_when_a_later_node_fails() {
        let cleaned = Arc::new(AtomicBool::new(false));
        let mut builder = TaskGraphBuilder::new("成功节点回滚");
        let step = builder.step("步骤");

        let cleaned_for_task = cleaned.clone();
        let successful = builder.add(
            step,
            "成功",
            (),
            task_fn(move |_: (), ctx| {
                let cleaned = cleaned_for_task.clone();
                async move {
                    ctx.on_rollback(move |_ctx| {
                        let cleaned = cleaned.clone();
                        async move {
                            cleaned.store(true, Ordering::SeqCst);
                        }
                    });
                    Ok(())
                }
            }),
        );
        let failing = builder.add(
            step,
            "失败",
            successful.clone(),
            task_fn(|_value: Arc<()>, _ctx| async move { Err::<(), _>(anyhow::anyhow!("失败")) }),
        );

        let graph = builder.build(failing).unwrap();
        let (result, _) = collect(graph).await;
        result.unwrap_err();
        assert!(cleaned.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn cleanups_do_not_run_after_a_successful_graph() {
        let cleaned = Arc::new(AtomicBool::new(false));
        let mut builder = TaskGraphBuilder::new("成功不回滚");
        let step = builder.step("步骤");

        let cleaned_for_task = cleaned.clone();
        let sink = builder.add(
            step,
            "成功",
            (),
            task_fn(move |_: (), ctx| {
                let cleaned = cleaned_for_task.clone();
                async move {
                    ctx.on_rollback(move |_ctx| {
                        let cleaned = cleaned.clone();
                        async move {
                            cleaned.store(true, Ordering::SeqCst);
                        }
                    });
                    Ok(())
                }
            }),
        );

        let graph = builder.build(sink).unwrap();
        let (result, _) = collect(graph).await;
        result.unwrap();
        assert!(!cleaned.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn a_warning_blocks_only_the_waiting_task() {
        let mut builder = TaskGraphBuilder::new("警告");
        let step = builder.step("步骤");
        let other_done = Arc::new(AtomicBool::new(false));
        let other_done_notify = Arc::new(tokio::sync::Notify::new());

        let warning = builder.add(
            step,
            "等待答复",
            (),
            task_fn(|_: (), ctx| async move { ctx.confirm("确认？").await }),
        );
        let other_done_for_task = other_done.clone();
        let notify_for_task = other_done_notify.clone();
        let other = builder.add(
            step,
            "其他任务",
            (),
            task_fn(move |_: (), _ctx| {
                let done = other_done_for_task.clone();
                let notify = notify_for_task.clone();
                async move {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    done.store(true, Ordering::SeqCst);
                    notify.notify_waiters();
                    Ok(())
                }
            }),
        );
        let sink = builder.add(
            step,
            "结束",
            (warning, other),
            task_fn(|_: (Arc<()>, Arc<()>), _ctx| async move { Ok(()) }),
        );

        let graph = builder.build(sink).unwrap();
        let mut receiver = graph.subscribe();
        let done = other_done.clone();
        let notify = other_done_notify.clone();
        let answer = tokio::spawn(async move {
            loop {
                match receiver.recv().await {
                    Some(TaskEvent::Warning { decision, .. }) => {
                        notify.notified().await;
                        assert!(done.load(Ordering::SeqCst), "其他任务应当继续执行");
                        decision.accept();
                        return;
                    }
                    Some(_) => {}
                    None => panic!("没有收到警告"),
                }
            }
        });

        let (result, _) = collect(graph).await;
        result.unwrap();
        answer.await.unwrap();
    }

    #[tokio::test]
    async fn aborting_a_warning_cancels_the_whole_graph() {
        let mut builder = TaskGraphBuilder::new("警告中止");
        let step = builder.step("步骤");
        let other_started = Arc::new(AtomicBool::new(false));

        let warning = builder.add(
            step,
            "等待答复",
            (),
            task_fn(|_: (), ctx| async move { ctx.confirm("确认？").await }),
        );
        let other_started_for_task = other_started.clone();
        let other = builder.add(
            step,
            "其他任务",
            (),
            task_fn(move |_: (), ctx| {
                let started = other_started_for_task.clone();
                async move {
                    started.store(true, Ordering::SeqCst);
                    ctx.cancel_handle().cancelled().await;
                    ctx.check_cancelled()
                }
            }),
        );
        let sink = builder.add(
            step,
            "结束",
            (warning, other),
            task_fn(|_: (Arc<()>, Arc<()>), _ctx| async move { Ok(()) }),
        );

        let graph = builder.build(sink).unwrap();
        let mut receiver = graph.subscribe();
        let answer = tokio::spawn(async move {
            loop {
                match receiver.recv().await {
                    Some(TaskEvent::Warning { decision, .. }) => {
                        decision.abort();
                        return;
                    }
                    Some(_) => {}
                    None => panic!("没有收到警告"),
                }
            }
        });

        let (result, _) = collect(graph).await;
        let error = result.unwrap_err();
        assert!(
            error
                .chain()
                .any(|cause| cause.downcast_ref::<crate::Cancelled>().is_some()),
            "{error:#}"
        );
        assert!(other_started.load(Ordering::SeqCst));
        answer.await.unwrap();
    }
}
