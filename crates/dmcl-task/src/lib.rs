//! 通用异步任务抽象 —— 与 Minecraft 无关，可单独复用。
//!
//! 设计目标：
//!
//! * **统一的 [`Task`] 接口** —— 任何"会花时间、需要报告进度"的动作都实现它；
//! * **可串联** —— [`TaskChain`] 把若干任务按顺序串起来，由 `then` 逐步追加；
//! * **实时进度** —— 任务通过 [`TaskContext`] 上报，前端用 [`TaskChain::subscribe`]
//!   订阅一条 [`TaskEvent`] 事件流（`broadcast` 通道），因此**一个任务可以上报任意多次**
//!   进度、也可以被多个前端（cli / tui / gui）同时消费；
//! * **快速失败** —— 链中任一步骤出错即中止后续步骤，并把错误原样返回给调用者；
//! * **可取消** —— [`CancelHandle`] 可以从任意线程触发取消。
//!
//! 典型用法：
//!
//! ```no_run
//! use dmcl_task::{CommandTask, TaskChain};
//!
//! # async fn run() -> anyhow::Result<()> {
//! let java = CommandTask::new("java").arg("-version").named("检查 Java");
//! let chain = TaskChain::new("启动游戏").then(java, "执行程序任务");
//!
//! // 事件流：一边跑一边打印
//! chain
//!     .run_with(|event| println!("{event}"))
//!     .await?;
//! # Ok(())
//! # }
//! ```
//!
//! 也可以手动订阅，自己控制消费节奏（GUI 场景尤其合适）：
//!
//! ```no_run
//! # use dmcl_task::{CommandTask, TaskChain};
//! # async fn run() -> anyhow::Result<()> {
//! let mut chain = TaskChain::new("下载游戏").then(CommandTask::new("true"), "准备");
//! let mut rx = chain.subscribe();
//! let handle = tokio::spawn(async move { chain.run().await });
//! while let Ok(event) = rx.recv().await {
//!     println!("{event}");
//! }
//! handle.await??;
//! # Ok(())
//! # }
//! ```

mod chain;
mod command;
mod context;
mod event;
mod progress;
mod task;

pub use chain::TaskChain;
pub use command::{CommandTask, OutputMode};
pub use context::{CancelHandle, Cancelled, TaskContext};
pub use event::TaskEvent;
pub use progress::{human_bytes, Progress};
pub use task::{task_fn, BoxFuture, FnTask, Task};
