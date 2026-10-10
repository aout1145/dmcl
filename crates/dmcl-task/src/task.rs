//! [`Task`] 接口本身，以及用闭包实现任务的适配器。

use std::future::Future;

use anyhow::Result;

use crate::context::TaskContext;

/// 一个可等待、可上报进度的异步动作。
///
/// 因为 trait 里要写 `async fn`，这里统一用装箱的 future（[`BoxFuture`]）表达，
/// 好处是 `Box<dyn Task>` 可以直接作为 trait 对象使用，而且不需要引入 `async-trait`
/// 依赖。实现者只多写一行 `Box::pin`：
///
/// ```no_run
/// use dmcl_task::{BoxFuture, Task, TaskContext};
///
/// struct Sleep;
///
/// impl Task for Sleep {
///     fn name(&self) -> &str {
///         "睡一会"
///     }
///
///     fn run<'a>(&'a self, ctx: TaskContext) -> BoxFuture<'a, anyhow::Result<()>> {
///         Box::pin(async move {
///             ctx.status("zzz");
///             tokio::time::sleep(std::time::Duration::from_millis(10)).await;
///             Ok(())
///         })
///     }
/// }
/// ```
pub trait Task: Send + Sync {
    /// 任务名，用于日志与默认的进度显示名。
    fn name(&self) -> &str;

    /// 执行任务。`ctx` 用于上报进度和检查取消；出错即表示该步骤失败。
    fn run<'a>(&'a self, ctx: TaskContext) -> BoxFuture<'a, Result<()>>;
}

impl Task for Box<dyn Task> {
    fn name(&self) -> &str {
        (**self).name()
    }

    fn run<'a>(&'a self, ctx: TaskContext) -> BoxFuture<'a, Result<()>> {
        (**self).run(ctx)
    }
}

/// 装箱的 future，用于在 [`Task`] 里表达 `async fn`。
pub type BoxFuture<'a, T> = std::pin::Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// 用闭包实现的任务，省去为每个小步骤定义一个 `struct`。
///
/// 一般通过 [`task_fn`] 构造。
pub struct FnTask<F> {
    name: String,
    run: F,
}

impl<F> FnTask<F> {
    pub fn new(name: impl Into<String>, run: F) -> Self {
        Self {
            name: name.into(),
            run,
        }
    }
}

impl<F, Fut> Task for FnTask<F>
where
    F: Fn(TaskContext) -> Fut + Send + Sync,
    Fut: Future<Output = Result<()>> + Send + 'static,
{
    fn name(&self) -> &str {
        &self.name
    }

    fn run<'a>(&'a self, ctx: TaskContext) -> BoxFuture<'a, Result<()>> {
        Box::pin((self.run)(ctx))
    }
}

/// 用闭包构造一个任务。
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
pub fn task_fn<F, Fut>(name: impl Into<String>, run: F) -> FnTask<F>
where
    F: Fn(TaskContext) -> Fut + Send + Sync,
    Fut: Future<Output = Result<()>> + Send + 'static,
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
        chain.run().await.unwrap();
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }
}
