//! 文件物化需要的外部输入，以及批量下载的薄封装。

use std::sync::Arc;

use anyhow::{Context as _, Result};
use dmcl_download::{DownloadContext, DownloadOptions};
use dmcl_mcmeta::source::DownloadSource;
use dmcl_task::{ProgressHandle, TaskContext};

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

    /// 用下载源的 http 客户端（同一连接池）建一个下载上下文。
    ///
    /// 同一个任务图里的多次下载应当共用它，才能共享连接 / 文件 / worker 预算
    /// 与全局进度。
    pub fn download_context(&self, progress: ProgressHandle) -> Arc<DownloadContext> {
        DownloadContext::new(
            self.source.client().clone(),
            self.download.clone(),
            progress,
        )
    }
}

/// 跑一批规划好的下载。
///
/// 必须运行在任务图节点里：内部会在当前 Step 下动态创建文件任务。
pub async fn download_planned(
    options: &GameFilesOptions,
    downloads: &Arc<DownloadContext>,
    name: &str,
    planned: Vec<PlannedFile>,
    ctx: &TaskContext,
) -> Result<()> {
    if planned.is_empty() {
        return Ok(());
    }
    let step = ctx.step();
    let items = planned
        .into_iter()
        .map(|file| file.into_item(options))
        .collect();
    let handles = downloads
        .prepare(ctx, step, items)
        .await
        .with_context(|| format!("{name}: 准备下载失败"))?;
    // 调用方接下来通常会读取这些文件（资源索引 / natives / 安装器），所以这里等到
    // 文件任务真正结束。纯下载步骤即使忽略句柄，也会由创建者自动 join。
    for handle in handles {
        handle
            .wait()
            .await
            .with_context(|| format!("{name}: 下载失败"))?;
    }
    Ok(())
}
