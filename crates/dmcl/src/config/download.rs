//! `download.*` 段。

use dmcl_mcmeta::source::MirrorPolicy;
use serde::{Deserialize, Serialize};

/// `download.*` 段。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DownloadConfig {
    /// 同时进行的下载连接数上限。
    pub max_tasks: usize,
    /// 跳过 `*.ogg` 资源（音乐与音效）。
    pub i_dont_need_music: bool,
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
            i_dont_need_music: false,
            mirror: MirrorPolicy::default(),
            chunk_threshold_mib: 8,
            chunk_size_mib: 4,
            max_chunks_per_file: 8,
            max_attempts: 3,
        }
    }
}
