//! 任务链。

use std::any::Any;
use std::marker::PhantomData;

use anyhow::Result;
use tokio::sync::broadcast;

use crate::context::{CancelHandle, TaskContext};
use crate::event::TaskEvent;
use crate::task::{BoxFuture, Task};

/// 把多个 [`Task`] 按顺序串起来的任务链。
///
/// * 类型参数 `I` 是整条链的输入、`O` 是最终输出；
/// * [`TaskChain::then`] 追加一个步骤，它接收上一步的输出 `O`、产出 `O2`，
///   于是链的类型变成 `TaskChain<I, O2>`（编译期保证对齐）；
/// * [`TaskChain::run`] 传入初始输入、返回最终输出；任一步失败即中止并返回错误；
/// * [`TaskChain::run_with`] 是「一边跑一边把事件交给回调」的便捷写法，同样返回最终输出；
/// * [`TaskChain::subscribe`] 拿到一条 [`TaskEvent`] 事件流。
///
/// 内部用 `Box<dyn Any>` 擦除中间值的具体类型，因此 `then` 可以在循环里动态追加；
/// 类型安全由 `then` / `run` 的泛型签名在编译期保证（擦除处的 `downcast` 必然成功）。
pub struct TaskChain<I, O>
where
    I: Send + 'static,
    O: Send + 'static,
{
    name: String,
    steps: Vec<Box<dyn AnyStep>>,
    events: broadcast::Sender<TaskEvent>,
    cancel: CancelHandle,
    marker: PhantomData<fn(I) -> O>,
}

/// 链里单个步骤的擦除形态：吃掉一个 `Box<dyn Any>`、吐出一个 `Box<dyn Any>`。
trait AnyStep: Send + Sync {
    fn name(&self) -> &str;
    fn run<'a>(
        &'a self,
        input: Box<dyn Any + Send>,
        ctx: TaskContext,
    ) -> BoxFuture<'a, Result<Box<dyn Any + Send>>>;
}

/// 把具体类型 `I -> O` 的任务擦成 [`AnyStep`]。
struct Erased<I, O, T> {
    task: T,
    name: String,
    marker: PhantomData<fn(I) -> O>,
}

impl<I, O, T> AnyStep for Erased<I, O, T>
where
    I: Send + 'static,
    O: Send + 'static,
    T: Task<I, O>,
{
    fn name(&self) -> &str {
        &self.name
    }

    fn run<'a>(
        &'a self,
        input: Box<dyn Any + Send>,
        ctx: TaskContext,
    ) -> BoxFuture<'a, Result<Box<dyn Any + Send>>> {
        Box::pin(async move {
            let input = *input.downcast::<I>().expect("任务链的输入类型没有对齐");
            let output = self.task.run(input, ctx).await?;
            Ok(Box::new(output) as Box<dyn Any + Send>)
        })
    }
}

impl TaskChain<(), ()> {
    /// 事件通道的默认容量（最多缓存多少条未消费的事件）。
    pub const DEFAULT_EVENT_CAPACITY: usize = 256;

    /// 新建一条**空**链（输入 `()`，输出 `()`），之后用 [`TaskChain::then`] 追加步骤。
    pub fn new(name: impl Into<String>) -> Self {
        Self::with_capacity(name, Self::DEFAULT_EVENT_CAPACITY)
    }

    /// 同 [`TaskChain::new`]，但可以指定事件通道容量。
    pub fn with_capacity(name: impl Into<String>, capacity: usize) -> Self {
        let (events, _) = broadcast::channel(capacity.max(1));
        TaskChain {
            name: name.into(),
            steps: Vec::new(),
            events,
            cancel: CancelHandle::default(),
            marker: PhantomData,
        }
    }
}

impl<I, O> TaskChain<I, O>
where
    I: Send + 'static,
    O: Send + 'static,
{
    /// 追加一个步骤：它接收当前链的输出 `O`，产出 `O2`。
    pub fn then<O2>(
        mut self,
        task: impl Task<O, O2> + 'static,
        name: impl Into<String>,
    ) -> TaskChain<I, O2>
    where
        O2: Send + 'static,
    {
        self.steps.push(Box::new(Erased::<O, O2, _> {
            task,
            name: name.into(),
            marker: PhantomData,
        }));
        TaskChain {
            name: self.name,
            steps: self.steps,
            events: self.events,
            cancel: self.cancel,
            marker: PhantomData,
        }
    }

    /// 追加一个已经装箱的步骤（用于运行时才能确定的步骤）。
    pub fn then_boxed<O2>(
        mut self,
        task: Box<dyn Task<O, O2>>,
        name: impl Into<String>,
    ) -> TaskChain<I, O2>
    where
        O2: Send + 'static,
    {
        self.steps.push(Box::new(Erased::<O, O2, _> {
            task,
            name: name.into(),
            marker: PhantomData,
        }));
        TaskChain {
            name: self.name,
            steps: self.steps,
            events: self.events,
            cancel: self.cancel,
            marker: PhantomData,
        }
    }

    /// 链名。
    pub fn name(&self) -> &str {
        &self.name
    }

    /// 步骤数。
    pub fn len(&self) -> usize {
        self.steps.len()
    }

    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    /// 每一步的名字（按执行顺序）。
    pub fn step_names(&self) -> impl Iterator<Item = &str> {
        self.steps.iter().map(|step| step.name())
    }

    /// 订阅进度事件流。
    ///
    /// 返回的接收者只收到**订阅之后**产生的事件；链结束（发送方被丢弃）时
    /// `recv()` 返回 `Err`。
    pub fn subscribe(&self) -> broadcast::Receiver<TaskEvent> {
        self.events.subscribe()
    }

    /// 取消句柄，可在运行期间从任意线程触发取消。
    pub fn cancel_handle(&self) -> CancelHandle {
        self.cancel.clone()
    }

    /// 顺序执行全部步骤，返回最后一个任务的输出。
    ///
    /// 失败时返回的错误里带有出错的步骤名（`with_context`），并已推送过一条
    /// [`TaskEvent::Failed`]。
    pub async fn run(&self, input: I) -> Result<O> {
        let steps = self.steps.len();
        let _ = self.events.send(TaskEvent::Started {
            name: self.name.clone(),
            steps,
        });

        let mut value: Box<dyn Any + Send> = Box::new(input);
        for (index, step) in self.steps.iter().enumerate() {
            if let Err(err) = self.cancel.check_cancelled() {
                self.report_failure(index, steps, step.name(), &err);
                return Err(err);
            }
            let _ = self.events.send(TaskEvent::StepStarted {
                step: index,
                steps,
                name: step.name().to_string(),
            });

            let ctx = TaskContext::new(
                self.events.clone(),
                self.cancel.clone(),
                index,
                steps,
                step.name(),
            );
            match step.run(value, ctx).await {
                Ok(output) => {
                    value = output;
                    let _ = self.events.send(TaskEvent::StepFinished {
                        step: index,
                        steps,
                        name: step.name().to_string(),
                    });
                }
                Err(err) => {
                    let err = err.context(format!("步骤「{}」失败", step.name()));
                    self.report_failure(index, steps, step.name(), &err);
                    return Err(err);
                }
            }
        }

        let _ = self.events.send(TaskEvent::Finished {
            name: self.name.clone(),
        });
        Ok(*value.downcast::<O>().expect("任务链的输出类型没有对齐"))
    }

    /// 运行任务链，边跑边把事件交给 `on_event`；最终输出原样返回。
    ///
    /// 即使 `on_event` 里做了耗时的事情（打印、刷新界面），也不会漏掉链的结果。
    pub async fn run_with<F>(&self, input: I, mut on_event: F) -> Result<O>
    where
        F: FnMut(TaskEvent),
    {
        let mut rx = self.subscribe();
        let mut closed = false;
        let running = self.run(input);
        tokio::pin!(running);

        loop {
            tokio::select! {
                biased;
                result = &mut running => {
                    // 链可能在没让出执行权的情况下就跑完了，此时通道里还留着事件：
                    // 先把它们全部交给回调，再返回结果，保证进度不丢。
                    while let Ok(event) = rx.try_recv() {
                        on_event(event);
                    }
                    return result;
                }
                event = rx.recv(), if !closed => match event {
                    Ok(event) => on_event(event),
                    // 通道空了或订阅者落后太多：继续等链自己结束即可
                    Err(_) => closed = true,
                },
            }
        }
    }

    fn report_failure(&self, step: usize, steps: usize, name: &str, error: &anyhow::Error) {
        let _ = self.events.send(TaskEvent::Failed {
            step,
            steps,
            name: name.to_string(),
            error: format!("{error:#}"),
        });
    }
}

impl<I, O> Task<I, O> for TaskChain<I, O>
where
    I: Send + 'static,
    O: Send + 'static,
{
    fn name(&self) -> &str {
        &self.name
    }

    /// 把整条子链当成一个步骤来跑。
    ///
    /// 子链有自己的事件通道，这里会把它的事件**转发**到父上下文，
    /// 所以"链套链"时父链的订阅者仍然能看到每一步的进度。
    fn run<'a>(&'a self, input: I, ctx: TaskContext) -> BoxFuture<'a, Result<O>> {
        Box::pin(async move {
            let mut rx = self.events.subscribe();
            let mut closed = false;
            let running = TaskChain::run(self, input);
            tokio::pin!(running);

            loop {
                tokio::select! {
                    biased;
                    result = &mut running => {
                        while let Ok(event) = rx.try_recv() {
                            forward_progress(&ctx, event);
                        }
                        return result;
                    }
                    event = rx.recv(), if !closed => match event {
                        Ok(event) => forward_progress(&ctx, event),
                        Err(_) => closed = true,
                    },
                }
            }
        })
    }
}

/// 只转发步骤内部上报的事件（日志 / 进度）：链路结构（开始 / 结束）由父链自己产生。
fn forward_progress(ctx: &TaskContext, event: TaskEvent) {
    match event {
        TaskEvent::Log { name, message, .. } => ctx.child(name).log(message),
        TaskEvent::Progress { name, progress, .. } => ctx.child(name).report(progress),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::{task_fn, task_fn_io};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    #[tokio::test]
    async fn steps_run_in_order() {
        let order: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let push = |value: &'static str| {
            let order = order.clone();
            task_fn(value, move |ctx: TaskContext| {
                let order = order.clone();
                async move {
                    order.lock().unwrap().push(value.to_string());
                    ctx.log("ok");
                    Ok(())
                }
            })
        };

        let chain = TaskChain::new("chain")
            .then(push("a"), "第一步")
            .then(push("b"), "第二步")
            .then(push("c"), "第三步");
        assert_eq!(chain.len(), 3);
        chain.run(()).await.unwrap();

        assert_eq!(*order.lock().unwrap(), vec!["a", "b", "c"]);
    }

    #[tokio::test]
    async fn values_flow_through_the_chain_to_the_caller() {
        let chain = TaskChain::new("流水线")
            .then(task_fn("起点", |_ctx| async move { Ok(1i32) }), "起点")
            .then(
                task_fn_io("加十", |n: i32, _ctx| async move { Ok(n + 10) }),
                "加十",
            );
        assert_eq!(chain.run(()).await.unwrap(), 11);
    }

    #[tokio::test]
    async fn a_failing_step_aborts_the_rest_and_names_itself() {
        let reached = Arc::new(AtomicBool::new(false));
        let flag = reached.clone();

        let chain = TaskChain::new("chain")
            .then(
                task_fn("boom", |_ctx| async move {
                    anyhow::bail!("故意失败");
                }),
                "会失败的一步",
            )
            .then(
                task_fn("later", move |_ctx| {
                    let flag = flag.clone();
                    async move {
                        flag.store(true, Ordering::SeqCst);
                        Ok(())
                    }
                }),
                "不该跑的一步",
            );

        let events = Arc::new(Mutex::new(Vec::new()));
        let collected = events.clone();
        let err = chain
            .run_with((), move |event| collected.lock().unwrap().push(event))
            .await
            .unwrap_err();

        assert!(!reached.load(Ordering::SeqCst), "失败后不应继续执行");
        let text = format!("{err:#}");
        assert!(text.contains("会失败的一步"), "{text}");
        assert!(text.contains("故意失败"), "{text}");

        let events = events.lock().unwrap();
        assert!(events
            .iter()
            .any(|event| matches!(event, TaskEvent::Failed { .. })));
        assert!(!events
            .iter()
            .any(|event| matches!(event, TaskEvent::Finished { .. })));
    }

    #[tokio::test]
    async fn events_describe_the_progress_of_each_step() {
        let chain = TaskChain::new("下载游戏").then(
            task_fn("download", |ctx| async move {
                ctx.progress(50, Some(100), "a.jar");
                ctx.progress(100, Some(100), "a.jar");
                Ok(())
            }),
            "下载任务",
        );

        let events = Arc::new(Mutex::new(Vec::new()));
        let collected = events.clone();
        chain
            .run_with((), move |event| collected.lock().unwrap().push(event))
            .await
            .unwrap();

        let events = events.lock().unwrap();
        assert_eq!(
            events[0],
            TaskEvent::Started {
                name: "下载游戏".to_string(),
                steps: 1
            }
        );
        // 一个任务可以上报多次进度：两次 Progress 都要在流里
        let progresses: Vec<_> = events.iter().filter_map(|event| event.progress()).collect();
        assert_eq!(progresses.len(), 2);
        assert_eq!(progresses[1].current, 100);
        assert!(matches!(events.last(), Some(TaskEvent::Finished { .. })));
    }

    #[tokio::test]
    async fn cancelling_before_run_stops_the_chain() {
        let chain =
            TaskChain::new("chain").then(task_fn("step", |_ctx| async move { Ok(()) }), "第一步");
        chain.cancel_handle().cancel();
        let err = chain.run(()).await.unwrap_err();
        assert!(err.downcast_ref::<crate::context::Cancelled>().is_some());
    }

    #[tokio::test]
    async fn nested_chains_forward_their_progress_to_the_parent() {
        let inner = TaskChain::new("子链").then(
            task_fn("下载", |ctx| async move {
                ctx.progress(1, Some(2), "a.jar");
                Ok(())
            }),
            "下载",
        );
        let outer = TaskChain::new("父链").then(inner, "子任务");

        let events = Arc::new(Mutex::new(Vec::new()));
        let collected = events.clone();
        outer
            .run_with((), move |event| collected.lock().unwrap().push(event))
            .await
            .unwrap();

        let events = events.lock().unwrap();
        // 子链里的进度必须出现在父链的事件流里，且带上子步骤的名字
        let progress: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                TaskEvent::Progress { name, progress, .. } => {
                    Some((name.clone(), progress.clone()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(progress.len(), 1, "{events:?}");
        assert_eq!(progress[0].0, "下载");
        assert_eq!(progress[0].1.current, 1);
    }

    #[tokio::test]
    async fn nested_chains_forward_their_logs_to_the_parent() {
        let inner = TaskChain::new("子链").then(
            task_fn("解压", |ctx| async move {
                ctx.log("正在解压 a.jar");
                Ok(())
            }),
            "解压",
        );
        let outer = TaskChain::new("父链").then(inner, "子任务");

        let events = Arc::new(Mutex::new(Vec::new()));
        let collected = events.clone();
        outer
            .run_with((), move |event| collected.lock().unwrap().push(event))
            .await
            .unwrap();

        let events = events.lock().unwrap();
        assert!(
            events.iter().any(|event| matches!(
                event,
                TaskEvent::Log { name, message, .. }
                    if name == "解压" && message == "正在解压 a.jar"
            )),
            "{events:?}"
        );
    }

    #[tokio::test]
    async fn a_chain_can_be_subscribed_from_two_front_ends() {
        let chain = TaskChain::new("chain").then(
            task_fn("step", |ctx| async move {
                ctx.log("hello");
                Ok(())
            }),
            "第一步",
        );
        let mut first = chain.subscribe();
        let mut second = chain.subscribe();
        let handle = tokio::spawn(async move { chain.run(()).await });

        let mut seen_from_first = 0;
        while first.recv().await.is_ok() {
            seen_from_first += 1;
        }
        handle.await.unwrap().unwrap();

        // 第一个订阅者已经消费完，第二个仍然持有自己那份拷贝
        assert!(seen_from_first >= 3);
        assert!(second.try_recv().is_ok());
    }
}
