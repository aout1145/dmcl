//! 通用异步任务图 —— 与 Minecraft 无关，可单独复用。
//!
//! 设计目标：
//!
//! * **只有任务 DAG** —— 节点之间只有依赖边，没有父子 / 链 / 组层级；
//! * **类型化数据流** —— 下游任务通过依赖注入直接拿到上游任务的返回值；
//! * **Step 只用于显示** —— 手动把节点分到若干 [`StepId`]，名称右侧显示任务计数；
//! * **运行时动态加节点** —— [`TaskContext::add_task`] 追加的动态节点自动成为创建者的
//!   依赖；
//! * **唯一最终输出** —— [`TaskGraphBuilder::build`] 显式指定最终节点，
//!   [`TaskGraph::run`] 返回它的输出；
//! * **FailFast 与回滚** —— 任一节点失败即全局取消，成功节点的回滚按完成逆序执行；
//! * **全局进度** —— 只保留整个图共享的下载字节数。
//!
//! 典型用法：
//!
//! ```no_run
//! use dmcl_task::{TaskGraphBuilder, task_fn};
//!
//! # async fn run() -> anyhow::Result<()> {
//! let mut builder = TaskGraphBuilder::new("安装 1.20.1");
//! let prepare = builder.step("准备");
//! let download = builder.step("下载");
//!
//! let version = builder.add(
//!     prepare,
//!     "获取版本",
//!     (),
//!     task_fn(|_: (), _ctx| async move { Ok(42u32) }),
//! );
//! let files = builder.add(
//!     download,
//!     "下载游戏文件",
//!     version,
//!     task_fn(|version: std::sync::Arc<u32>, _ctx| async move {
//!         Ok(*version)
//!     }),
//! );
//!
//! let graph = builder.build(files)?;
//! graph.run(|event| println!("{event}")).await?;
//! # Ok(())
//! # }
//! ```

mod bus;
mod command;
mod context;
mod deps;
mod event;
mod graph;
mod progress;
mod runtime;
mod task;
mod warning;

pub use bus::EventReceiver;
pub use command::{CommandTask, OutputMode};
pub use context::{AddTaskError, CancelHandle, Cancelled, TaskContext};
pub use deps::{Dependency, Deps, DynamicHandle, Handle, TaskHandle, WaitableTaskHandle};
pub use event::{TaskEvent, TaskRef, TaskStatus};
pub use graph::{GraphError, StepDef, StepId, TaskGraph, TaskGraphBuilder};
pub use progress::{human_bytes, ProgressHandle, ProgressSnapshot};
pub use task::{task_fn, FnTask, Task};
pub use warning::WarningDecision;
