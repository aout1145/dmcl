//! `download.*` 段与下载 Options 转换。

use dmcl_download::{DownloadOptions, DEFAULT_USER_AGENT};
use dmcl_launch::{LAUNCHER_NAME, LAUNCHER_VERSION};
use dmcl_mcmeta::source::{DownloadSource, MirrorPolicy};
use serde::{Deserialize, Serialize};

use super::Config;

/// 1 MiB。
const MIB: u64 = 1024 * 1024;

/// `download.*` 段。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DownloadConfig {
    /// 同时进行的下载连接数上限。
    pub max_tasks: usize,
    /// 跳过 `*.ogg` 资源（音乐与音效）。
    pub skip_music: bool,
    /// 镜像使用策略（仅官方 / 官方优先 / 镜像优先）。
    pub mirror: MirrorPolicy,
    /// 超过这个大小（MiB）就分块下载。
    pub chunk_threshold_mib: u64,
    /// 每块大小（MiB）。
    pub chunk_size_mib: u64,
    /// 单个文件最多并发几块。
    pub max_chunks_per_file: usize,
    /// 单个文件最多尝试几次。
    pub max_attempts: u32,
}

impl Default for DownloadConfig {
    fn default() -> Self {
        Self {
            max_tasks: 64,
            skip_music: false,
            mirror: MirrorPolicy::default(),
            chunk_threshold_mib: 8,
            chunk_size_mib: 4,
            max_chunks_per_file: 8,
            max_attempts: 3,
        }
    }
}

impl Config {
    /// 按配置造一个下载源（镜像策略 + 重试次数 + User-Agent）。
    pub fn download_source(&self) -> DownloadSource {
        DownloadSource::from_policy(self.download.mirror)
            .with_user_agent(format!("{LAUNCHER_NAME}/{LAUNCHER_VERSION}"))
            .with_max_attempts(self.download.max_attempts)
    }

    /// 配置文件里的下载策略 → [`dmcl_download::DownloadOptions`]。
    pub fn download_options(&self) -> DownloadOptions {
        let mut options = DownloadOptions::with_max_connections(self.download.max_tasks)
            .with_chunking(
                self.download.chunk_threshold_mib.max(1) * MIB,
                self.download.chunk_size_mib.max(1) * MIB,
            )
            .with_max_attempts(self.download.max_attempts)
            .with_user_agent(DEFAULT_USER_AGENT);
        options.max_chunks_per_file = self.download.max_chunks_per_file.max(1);
        options
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use dmcl_mcmeta::source::MirrorPolicy;

    #[test]
    fn download_defaults_are_official_first() {
        let config = Config::default();
        assert_eq!(config.download.mirror, MirrorPolicy::OfficialFirst);
        assert!(!config.download.skip_music);
        assert_eq!(config.download.max_tasks, 64);
    }

    #[test]
    fn download_options_follow_the_config() {
        let mut config = Config::default();
        config.download.max_tasks = 8;
        config.download.chunk_threshold_mib = 2;
        config.download.chunk_size_mib = 1;
        config.download.max_chunks_per_file = 3;
        config.download.max_attempts = 5;

        let download = config.download_options();
        assert_eq!(download.max_connections, 8);
        assert_eq!(download.chunk_threshold, 2 * MIB);
        assert_eq!(download.chunk_size, MIB);
        assert_eq!(download.max_chunks_per_file, 3);
        assert_eq!(download.max_attempts, 5);
    }
}
