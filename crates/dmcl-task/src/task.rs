//! [`Task`] 接口本身，以及用闭包实现任务的适配器。

use std::future::Future;
use std::marker::PhantomData;

use anyhow::Result;

use crate::context::TaskContext;

/// 一个可等待的异步动作：接收输入 `I`，产出输出 `O`。
///
/// 输入通过依赖注入传入（见 [`crate::Deps`]）；没有依赖时 `I = ()`，
/// 有一个依赖时 `I = Arc<T>`，多个依赖时 `I = (Arc<A>, Arc<B>, …)`。
///
/// ```no_run
/// use dmcl_task::{Task, TaskContext};
///
/// struct AddOne;
///
/// #[async_trait::async_trait]
/// impl Task<i32, i32> for AddOne {
///     async fn run(&self, input: i32, ctx: TaskContext) -> anyhow::Result<i32> {
///         ctx.log(format!("输入 {input}"));
///         Ok(input + 1)
///     }
/// }
/// ```
#[async_trait::async_trait]
pub trait Task<I, O>: Send + Sync
where
    I: Send + 'static,
    O: Send + 'static,
{
    /// 执行任务；`ctx` 用于日志、取消、动态节点与回滚登记。
    async fn run(&self, input: I, ctx: TaskContext) -> Result<O>;
}

#[async_trait::async_trait]
impl<I, O> Task<I, O> for Box<dyn Task<I, O>>
where
    I: Send + 'static,
    O: Send + 'static,
{
    async fn run(&self, input: I, ctx: TaskContext) -> Result<O> {
        (**self).run(input, ctx).await
    }
}

/// 用闭包实现的任务，省去为每个小步骤定义一个 `struct`。
///
/// 一般通过 [`task_fn`] 构造。
pub struct FnTask<I, O, F> {
    run: F,
    marker: PhantomData<fn(I) -> O>,
}

impl<I, O, F> FnTask<I, O, F> {
    pub fn new(run: F) -> Self {
        Self {
            run,
            marker: PhantomData,
        }
    }
}

#[async_trait::async_trait]
impl<I, O, F, Fut> Task<I, O> for FnTask<I, O, F>
where
    I: Send + 'static,
    O: Send + 'static,
    F: Fn(I, TaskContext) -> Fut + Send + Sync,
    Fut: Future<Output = Result<O>> + Send + 'static,
{
    async fn run(&self, input: I, ctx: TaskContext) -> Result<O> {
        (self.run)(input, ctx).await
    }
}

/// 用闭包构造任务：闭包统一接收依赖注入的输入与 [`TaskContext`]。
///
/// 没有依赖时输入是 `()`；有一个依赖时是 `Arc<T>`；多个依赖时是
/// `(Arc<A>, Arc<B>, …)`。
pub fn task_fn<I, O, F, Fut>(run: F) -> FnTask<I, O, F>
where
    I: Send + 'static,
    O: Send + 'static,
    F: Fn(I, TaskContext) -> Fut + Send + Sync,
    Fut: Future<Output = Result<O>> + Send + 'static,
{
    FnTask::new(run)
}
