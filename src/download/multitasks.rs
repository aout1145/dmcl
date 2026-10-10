//! Concurrent downloads (was `dmcl.download.multitasks`).
//!
//! The D version used a `std.parallelism.TaskPool` with
//! `config.download_max_tasks` worker threads. Here each download is a
//! `tokio` task and the concurrency limit is enforced with a
//! [`tokio::sync::Semaphore`] carrying the same `download.max_tasks` value.
//! Because the runtime is multi-threaded (`#[tokio::main]` defaults to the
//! multi-thread scheduler), downloads genuinely run on several threads.
//!
//! Enqueueing never blocks: the permit is acquired *inside* the spawned task,
//! so `download_file` behaves exactly like the original `taskPool.put(...)`,
//! while only `download.max_tasks` downloads are ever in flight.

use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::Result;
use tokio::sync::Semaphore;
use tokio::task::JoinHandle;

use crate::config::with_config;
use crate::utils::{check_file, download_safe, get_path};

/// `DownloadFileMeta`.
#[derive(Debug, Clone)]
pub struct DownloadFileMeta {
    pub url: String,
    /// Where the file goes — an absolute path, resolved on enqueue.
    pub save_to: std::path::PathBuf,
    pub sha1: Option<String>,
}

impl DownloadFileMeta {
    pub fn new(
        url: impl Into<String>,
        save_to: impl Into<std::path::PathBuf>,
        sha1: Option<String>,
    ) -> Self {
        Self {
            url: url.into(),
            save_to: save_to.into(),
            sha1,
        }
    }
}

/// Owns the in-flight download tasks.
pub struct DownloadManager {
    semaphore: Arc<Semaphore>,
    handles: Mutex<Vec<JoinHandle<()>>>,
}

impl DownloadManager {
    pub fn new(max_tasks: usize) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(max_tasks.max(1))),
            handles: Mutex::new(Vec::new()),
        }
    }

    /// `downloadFile` — make the destination absolute and queue the download.
    pub fn download(&self, mut meta: DownloadFileMeta) {
        meta.save_to = get_path(&meta.save_to);
        let semaphore = self.semaphore.clone();
        let handle = tokio::spawn(async move {
            let Ok(_permit) = semaphore.acquire().await else {
                return;
            };
            if let Err(err) = download_one(&meta).await {
                eprintln!("download failed: {err:#}");
            }
        });
        if let Ok(mut guard) = self.handles.lock() {
            guard.push(handle);
        }
    }

    /// `waitDownloads` — join every task queued so far.
    pub async fn wait(&self) {
        let handles: Vec<JoinHandle<()>> = match self.handles.lock() {
            Ok(mut guard) => std::mem::take(&mut *guard),
            Err(_) => Vec::new(),
        };
        for handle in handles {
            let _ = handle.await;
        }
    }
}

/// `downloadFiles_downloadFunc`.
async fn download_one(meta: &DownloadFileMeta) -> Result<()> {
    if !check_file(&meta.save_to, meta.sha1.as_deref()).await {
        download_safe(&meta.url, &meta.save_to, meta.sha1.as_deref()).await?;
        println!("{}", base_name(&meta.save_to));
    }
    Ok(())
}

pub(crate) fn base_name(path: &Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

/// Global manager, created lazily on first use (like the original `taskPool`).
static MANAGER: OnceLock<DownloadManager> = OnceLock::new();

pub fn manager() -> &'static DownloadManager {
    MANAGER.get_or_init(|| DownloadManager::new(with_config(|c| c.download.max_tasks)))
}

/// `downloadFile`.
///
/// Must be called from within a tokio runtime (the whole launcher runs inside
/// one). For an embedded/GUI use-case, build your own [`DownloadManager`].
pub fn download_file(meta: DownloadFileMeta) {
    manager().download(meta);
}

/// `waitDownloads`.
pub async fn wait_downloads() {
    manager().wait().await;
}
