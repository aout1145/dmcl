//! JRE 下载需要的传输配置。

use dmcl_download::DownloadOptions;
use dmcl_mcmeta::source::DownloadSource;

/// JRE 下载需要的传输配置。
#[derive(Debug, Clone)]
pub struct JavaRuntimeOptions {
    pub source: DownloadSource,
    pub download: DownloadOptions,
}
