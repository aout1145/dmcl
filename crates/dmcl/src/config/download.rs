//! `download.*` 段与下载 Options 转换。

use dmcl_download::{DownloadOptions, DEFAULT_USER_AGENT};
use dmcl_launch::{LAUNCHER_NAME, LAUNCHER_VERSION};
use dmcl_mcmeta::source::{DownloadSource, MirrorPolicy};
use serde::{Deserialize, Serialize};

use super::{Config, JavaChoice};

/// 1 MiB。
const MIB: u64 = 1024 * 1024;

/// `download.*` 段。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DownloadConfig {
    /// 同时进行的 HTTP 请求数上限，也是全局 in-flight chunk 上限。
    pub max_connections: usize,
    /// 同时活跃的文件数上限。
    pub max_active_files: usize,
    /// CPU 密集型任务（预检 / 校验）的并发上限。
    pub max_workers: usize,
    /// 跳过 `*.ogg` 资源（音乐与音效）。
    pub skip_music: bool,
    /// 镜像使用策略（仅官方 / 官方优先 / 镜像优先）。
    pub mirror: MirrorPolicy,
    /// 超过这个大小（MiB）就分块下载。
    pub chunk_threshold_mib: u64,
    /// 每块大小（MiB）。
    pub chunk_size_mib: u64,
    /// 单个文件失败后最多重试几次（不含第一次尝试）。
    pub max_retries: u32,
    /// 安装器 processor 用哪个 Java，或自动选择。
    pub java: JavaChoice,
}

impl Default for DownloadConfig {
    fn default() -> Self {
        let defaults = DownloadOptions::default();
        Self {
            max_connections: defaults.max_connections,
            max_active_files: defaults.max_active_files,
            max_workers: defaults.max_workers,
            skip_music: false,
            mirror: MirrorPolicy::default(),
            chunk_threshold_mib: 8,
            chunk_size_mib: 4,
            max_retries: defaults.max_retries,
            java: JavaChoice::Auto,
        }
    }
}

impl Config {
    /// 按配置造一个下载源（镜像策略 + 重试次数 + User-Agent）。
    pub fn download_source(&self) -> DownloadSource {
        DownloadSource::from_policy(self.download.mirror)
            .with_user_agent(format!("{LAUNCHER_NAME}/{LAUNCHER_VERSION}"))
            .with_max_retries(self.download.max_retries)
    }

    /// 配置文件里的下载策略 → [`dmcl_download::DownloadOptions`]。
    pub fn download_options(&self) -> DownloadOptions {
        DownloadOptions::with_max_connections(self.download.max_connections)
            .with_chunking(
                self.download.chunk_threshold_mib.max(1) * MIB,
                self.download.chunk_size_mib.max(1) * MIB,
            )
            .with_max_active_files(self.download.max_active_files)
            .with_max_workers(self.download.max_workers)
            .with_max_retries(self.download.max_retries)
            .with_user_agent(DEFAULT_USER_AGENT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn download_options_follow_the_config() {
        let mut config = Config::default();
        config.download.max_connections = 8;
        config.download.max_active_files = 4;
        config.download.max_workers = 2;
        config.download.chunk_threshold_mib = 2;
        config.download.chunk_size_mib = 1;
        config.download.max_retries = 5;

        let download = config.download_options();
        assert_eq!(download.max_connections, 8);
        assert_eq!(download.max_active_files, 4);
        assert_eq!(download.max_workers, 2);
        assert_eq!(download.chunk_threshold, 2 * MIB);
        assert_eq!(download.chunk_size, MIB);
        assert_eq!(download.max_retries, 5);
    }
}
