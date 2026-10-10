//! [`Task`] 接口本身，以及用闭包实现任务的适配器。
//!
//! 任务不是"无产出的副作用动作"，而是**接收一个输入、产出一个输出**的异步步骤；
//! [`TaskChain`] 用 `then` 把上一个任务的输出接到下一个任务的输入上，直到最后一个
//! 任务把最终结果返回给调用者（见 `TaskChain` 的 `run` / `run_with`）。

use std::future::Future;
use std::marker::PhantomData;

use anyhow::Result;

use crate::context::TaskContext;

/// 一个可等待、可上报进度的异步动作：接收输入 `I`，产出输出 `O`。
///
/// 因为 trait 里要写 `async fn`，这里统一用装箱的 future（[`BoxFuture`]）表达，
/// 好处是 `Box<dyn Task<I, O>>` 可以直接作为 trait 对象使用，而且不需要引入
/// `async-trait` 依赖。实现者只多写一行 `Box::pin`：
///
/// ```no_run
/// use dmcl_task::{BoxFuture, Task, TaskContext};
///
/// struct AddOne;
///
/// impl Task<i32, i32> for AddOne {
///     fn name(&self) -> &str {
///         "加一"
///     }
///
///     fn run<'a>(&'a self, input: i32, ctx: TaskContext) -> BoxFuture<'a, anyhow::Result<i32>> {
///         Box::pin(async move {
///             ctx.status(format!("输入 {input}"));
///             Ok(input + 1)
///         })
///     }
/// }
/// ```
pub trait Task<I, O>: Send + Sync
where
    I: Send + 'static,
    O: Send + 'static,
{
    /// 任务名，用于日志与默认的进度显示名。
    fn name(&self) -> &str;

    /// 执行任务。`input` 是上一步传下来的值（首个任务通常是 `()`），
    /// `ctx` 用于上报进度和检查取消；出错即表示该步骤失败。
    fn run<'a>(&'a self, input: I, ctx: TaskContext) -> BoxFuture<'a, Result<O>>;
}

impl<I, O> Task<I, O> for Box<dyn Task<I, O>>
where
    I: Send + 'static,
    O: Send + 'static,
{
    fn name(&self) -> &str {
        (**self).name()
    }

    fn run<'a>(&'a self, input: I, ctx: TaskContext) -> BoxFuture<'a, Result<O>> {
        (**self).run(input, ctx)
    }
}

/// 装箱的 future，用于在 [`Task`] 里表达 `async fn`。
pub type BoxFuture<'a, T> = std::pin::Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// 用闭包实现的任务，省去为每个小步骤定义一个 `struct`。
///
/// 一般通过 [`task_fn`]（无输入）或 [`task_fn_io`]（带输入）构造。
pub struct FnTask<I, O, F> {
    name: String,
    run: F,
    marker: PhantomData<fn(I) -> O>,
}

impl<I, O, F> FnTask<I, O, F> {
    pub fn new(name: impl Into<String>, run: F) -> Self {
        Self {
            name: name.into(),
            run,
            marker: PhantomData,
        }
    }
}

impl<I, O, F, Fut> Task<I, O> for FnTask<I, O, F>
where
    I: Send + 'static,
    O: Send + 'static,
    F: Fn(I, TaskContext) -> Fut + Send + Sync,
    Fut: Future<Output = Result<O>> + Send + 'static,
{
    fn name(&self) -> &str {
        &self.name
    }

    fn run<'a>(&'a self, input: I, ctx: TaskContext) -> BoxFuture<'a, Result<O>> {
        Box::pin((self.run)(input, ctx))
    }
}

/// 用闭包构造一个**无输入**的任务（输入为 `()`）。
///
/// 这是最常见的情形：配置通过 `Arc<Options>` 注入、闭包只关心上报进度。
///
/// ```no_run
/// # use dmcl_task::{task_fn, TaskChain};
/// # async fn run() -> anyhow::Result<()> {
/// let chain = TaskChain::new("安装").then(
///     task_fn("写配置文件", |ctx| async move {
///         ctx.status("writing");
///         Ok(())
///     }),
///     "写配置文件",
/// );
/// # let _ = chain;
/// # Ok(())
/// # }
/// ```
pub fn task_fn<O, F, Fut>(
    name: impl Into<String>,
    run: F,
) -> FnTask<(), O, impl Fn((), TaskContext) -> Fut>
where
    O: Send + 'static,
    F: Fn(TaskContext) -> Fut + Send + Sync,
    Fut: Future<Output = Result<O>> + Send + 'static,
{
    FnTask {
        name: name.into(),
        run: move |_, ctx| (run)(ctx),
        marker: PhantomData,
    }
}

/// 用闭包构造一个**带输入**的任务：闭包拿到上一步传下来的 `input`。
///
/// ```no_run
/// # use dmcl_task::{task_fn, task_fn_io, TaskChain};
/// # async fn run() -> anyhow::Result<()> {
/// let chain = TaskChain::new("数字")
///     .then(task_fn("起点", |_ctx| async move { Ok(5i32) }), "起点")
///     .then(
///         task_fn_io("翻倍", |input: i32, _ctx| async move { Ok(input * 2) }),
///         "翻倍",
///     );
/// # let _ = chain;
/// # Ok(())
/// # }
/// ```
pub fn task_fn_io<I, O, F, Fut>(name: impl Into<String>, run: F) -> FnTask<I, O, F>
where
    I: Send + 'static,
    O: Send + 'static,
    F: Fn(I, TaskContext) -> Fut + Send + Sync,
    Fut: Future<Output = Result<O>> + Send + 'static,
{
    FnTask::new(name, run)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chain::TaskChain;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[tokio::test]
    async fn closure_tasks_run_and_report() {
        let counter = Arc::new(AtomicUsize::new(0));
        let seen = counter.clone();
        let chain = TaskChain::new("t").then(
            task_fn("count", move |ctx| {
                let seen = seen.clone();
                async move {
                    seen.fetch_add(1, Ordering::SeqCst);
                    ctx.status("counted");
                    Ok(())
                }
            }),
            "count",
        );
        chain.run(()).await.unwrap();
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_task_with_input_receives_the_previous_output() {
        let chain = TaskChain::new("数字")
            .then(task_fn("起点", |_ctx| async move { Ok(10i32) }), "起点")
            .then(
                task_fn_io("加一", |n: i32, _ctx| async move { Ok(n + 1) }),
                "加一",
            )
            .then(
                task_fn_io("翻倍", |n: i32, _ctx| async move { Ok(n * 2) }),
                "翻倍",
            );
        let result = chain.run(()).await.unwrap();
        assert_eq!(result, 22);
    }
}
