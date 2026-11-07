//! JRE 下载需要的传输配置。

use std::sync::Arc;

use dmcl_download::{DownloadContext, DownloadOptions};
use dmcl_mcmeta::source::DownloadSource;
use dmcl_task::ProgressHandle;

/// JRE 下载需要的传输配置。
#[derive(Debug, Clone)]
pub struct JavaRuntimeOptions {
    pub source: DownloadSource,
    pub download: DownloadOptions,
}

impl JavaRuntimeOptions {
    /// 用下载源的 http 客户端建一个下载上下文。
    pub fn download_context(&self, progress: ProgressHandle) -> Arc<DownloadContext> {
        DownloadContext::new(
            self.source.client().clone(),
            self.download.clone(),
            progress,
        )
    }
}
