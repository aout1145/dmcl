//! 文件下载：单文件任务 + 共享并发预算 + 全局进度。
//!
//! 每个文件是一个独立的 [`dmcl_task::Task`]，由下载准备节点动态加入任务图：
//!
//! ```no_run
//! use dmcl_download::{DownloadContext, DownloadItem, DownloadOptions};
//! use dmcl_task::{StepId, TaskContext};
//!
//! # async fn run(ctx: TaskContext, step: StepId, items: Vec<DownloadItem>) -> anyhow::Result<()> {
//! let downloads = DownloadContext::new(
//!     reqwest::Client::new(),
//!     DownloadOptions::default(),
//!     ctx.progress_handle(),
//! );
//! // 在准备节点里：预检 → add_total → ctx.add_task 动态创建文件任务
//! downloads.prepare(&ctx, step, items).await?;
//! # Ok(())
//! # }
//! ```
//!
//! 设计要点：
//!
//! * **每个文件一个 DAG 节点**：文件任务由准备节点 `add_task` 创建，自动 join；
//! * **三层并发预算**：`max_active_files`（文件）、`max_connections`（HTTP 请求 /
//!   分块）、`max_workers`（预检与校验等 CPU 任务）；
//! * **分块共享全局连接**：每块占一个 `max_connections` 许可，单个大文件可以占满全部
//!   连接，多个文件共享同一预算；
//! * **本地判断也并发**：预检走 [`files_ok_with`]，与下载后的校验共享 worker 预算；
//! * **先写 `.part` 再改名**：校验通过才 rename，失败直接让节点失败，由 DAG FailFast
//!   处理；
//! * **换源重试**：每个下载项可带一串候选地址，第 `n` 次尝试轮换一个（A B A …）；
//! * **指数退避**：重试前按指数等待；429 等限流响应优先遵循 `Retry-After`（或 bmclapi 的
//!   `x-ratelimit-reset-short`）；
//! * **不做断点续传**：失败即整份重来（首次之外最多重试 [`DownloadOptions::max_retries`] 次）。

mod hash;
mod item;
mod options;
mod task;

pub use hash::{
    file_ok, files_ok, files_ok_with, hash_bytes, hash_file, Checksum, FileCheck, HashAlgorithm,
};
pub use item::DownloadItem;
pub use options::{DownloadOptions, DEFAULT_USER_AGENT};
pub use task::DownloadContext;
