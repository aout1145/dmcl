//! 文件下载：多文件并发 + 大文件分块并发 + 进度上报。
//!
//! 这里的 [`DownloadTask`] 是一个 [`dmcl_task::Task`]，因此可以直接塞进任务链，
//! 由前端（cli / tui / gui）订阅事件流显示整体进度：
//!
//! ```no_run
//! use dmcl_download::{DownloadItem, DownloadOptions, DownloadTask};
//! use dmcl_task::TaskChain;
//!
//! # async fn run() -> anyhow::Result<()> {
//! let mut downloads = DownloadTask::new(DownloadOptions::default());
//! downloads.push(DownloadItem::new("https://example.com/a.jar", "/tmp/a.jar"));
//!
//! let chain = TaskChain::new("下载").then(downloads, "下载文件");
//! // 这条链的最终输出就是 DownloadReport
//! let _report: dmcl_download::DownloadReport = chain.run_with((), |event| println!("{event}")).await?;
//! # Ok(())
//! # }
//! ```
//!
//! 设计要点：
//!
//! * **并发靠 tokio**：整个启动器跑在多线程运行时上，因此这些并发任务是真的并行执行；
//!   同时进行的连接数由 [`DownloadOptions::max_connections`] 通过信号量限制。
//! * **分块**：已知大小且超过 [`DownloadOptions::chunk_threshold`] 的文件按
//!   `Range` 请求切成若干块并发下载，单文件并发块数由
//!   [`DownloadOptions::max_chunks_per_file`] 限制。
//! * **先写 `.part` 再改名**：校验通过（给了校验值就比对摘要，否则比对大小）才改名为最终文件，
//!   因此中断 / 失败不会留下一个"看起来完整"的坏文件。
//! * **不做断点续传**：失败即整份重来（最多 [`DownloadOptions::max_attempts`] 次）。

mod hash;
mod item;
mod options;
mod report;
mod task;

pub use hash::{
    check_file, file_matches, hash_bytes, hash_file, sha1_hex, Checksum, HashAlgorithm,
};
pub use item::DownloadItem;
pub use options::{DownloadOptions, DEFAULT_USER_AGENT};
pub use report::{DownloadFailure, DownloadReport};
pub use task::DownloadTask;
