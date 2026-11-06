//! 文件物化需要的外部输入，以及批量下载的薄封装。

use anyhow::Result;
use dmcl_download::{DownloadItem, DownloadOptions, DownloadReport, DownloadTask};
use dmcl_mcmeta::source::DownloadSource;
use dmcl_task::TaskContext;

use crate::plan::PlannedFile;

/// 文件物化需要的外部输入。
#[derive(Debug, Clone)]
pub struct GameFilesOptions {
    pub source: DownloadSource,
    pub download: DownloadOptions,
    pub skip_music: bool,
}

impl GameFilesOptions {
    pub fn new(source: DownloadSource) -> Self {
        Self {
            source,
            download: DownloadOptions::default(),
            skip_music: false,
        }
    }

    pub fn with_download(mut self, download: DownloadOptions) -> Self {
        self.download = download;
        self
    }

    pub fn with_skip_music(mut self, skip_music: bool) -> Self {
        self.skip_music = skip_music;
        self
    }

    /// 复用下载源的 http 客户端（同一连接池）。
    pub fn download_task(&self, items: Vec<DownloadItem>) -> DownloadTask {
        let mut task =
            DownloadTask::new(self.download.clone()).with_client(self.source.client().clone());
        task.extend(items);
        task
    }
}

/// 跑一批规划好的下载，并检查结果。
///
/// 只要有文件失败就返回错误 —— 安装不能「少几个文件也当成功」。
pub async fn download_planned(
    options: &GameFilesOptions,
    name: &str,
    planned: Vec<PlannedFile>,
    ctx: &TaskContext,
) -> Result<DownloadReport> {
    if planned.is_empty() {
        return Ok(DownloadReport::default());
    }
    let items = planned
        .into_iter()
        .map(|file| file.into_item(options))
        .collect();
    let task = options.download_task(items);
    let report = task.execute(ctx).await?;
    if !report.is_success() {
        anyhow::bail!(
            "{name}: {} 个文件下载失败：\n{}",
            report.failed.len(),
            report.failure_summary(5)
        );
    }
    Ok(report)
}
