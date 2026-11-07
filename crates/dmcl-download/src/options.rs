//! 下载的并发与分块策略。

use std::time::Duration;

/// 默认的 User-Agent（与其它 crate 保持一致，但各自独立，避免相互依赖）。
pub const DEFAULT_USER_AGENT: &str = concat!("dmcl/", env!("CARGO_PKG_VERSION"));

/// 第 1 次重试的基础等待时间。
const DEFAULT_RETRY_BASE_DELAY: Duration = Duration::from_secs(1);
/// 指数退避的上限。
const DEFAULT_RETRY_MAX_DELAY: Duration = Duration::from_secs(60);

/// 下载的并发与分块策略。
///
/// 全部由调用者提供 —— 这个 crate 不读任何配置文件。
#[derive(Debug, Clone)]
pub struct DownloadOptions {
    /// 同时进行的 HTTP 请求数上限，也是全局 in-flight chunk 上限。
    pub max_connections: usize,
    /// 同时活跃的文件数上限。
    pub max_active_files: usize,
    /// CPU 密集型任务（预检 / 校验）的并发上限。
    pub max_workers: usize,
    /// 超过这个字节数就分块下载。
    pub chunk_threshold: u64,
    /// 每块的大小。
    pub chunk_size: u64,
    /// 单个文件失败后最多重试几次（不含第一次尝试）。
    pub max_retries: u32,
    /// 第 1 次重试前等待多久；之后每次翻倍。
    ///
    /// 服务器返回 `Retry-After` 时以响应头为准，不使用这里的退避。
    pub retry_base_delay: Duration,
    /// 指数退避的等待上限（不影响 `Retry-After`）。
    pub retry_max_delay: Duration,
    pub user_agent: String,
}

impl Default for DownloadOptions {
    fn default() -> Self {
        Self {
            max_connections: 64,
            max_active_files: 64,
            max_workers: std::thread::available_parallelism()
                .map(|threads| threads.get())
                .unwrap_or(1),
            chunk_threshold: 8 * 1024 * 1024,
            chunk_size: 4 * 1024 * 1024,
            max_retries: 3,
            retry_base_delay: DEFAULT_RETRY_BASE_DELAY,
            retry_max_delay: DEFAULT_RETRY_MAX_DELAY,
            user_agent: DEFAULT_USER_AGENT.to_string(),
        }
    }
}

impl DownloadOptions {
    /// 只设置连接数、其余取默认值。
    pub fn with_max_connections(max_connections: usize) -> Self {
        Self {
            max_connections: max_connections.max(1),
            ..Self::default()
        }
    }

    /// 链式设置连接数上限。
    pub fn with_connection_limit(mut self, max_connections: usize) -> Self {
        self.max_connections = max_connections;
        self
    }

    /// 关掉分块（全部单连接下载）。
    pub fn without_chunking(mut self) -> Self {
        self.chunk_threshold = u64::MAX;
        self
    }

    pub fn with_chunking(mut self, threshold: u64, chunk_size: u64) -> Self {
        self.chunk_threshold = threshold;
        self.chunk_size = chunk_size;
        self
    }

    pub fn with_max_active_files(mut self, max_active_files: usize) -> Self {
        self.max_active_files = max_active_files;
        self
    }

    pub fn with_max_workers(mut self, max_workers: usize) -> Self {
        self.max_workers = max_workers;
        self
    }

    pub fn with_max_retries(mut self, max_retries: u32) -> Self {
        self.max_retries = max_retries;
        self
    }

    /// 设置指数退避：第 1 次重试等 `base`，之后翻倍，最长不超过 `max`。
    pub fn with_retry_delay(mut self, base: Duration, max: Duration) -> Self {
        self.retry_base_delay = base;
        self.retry_max_delay = max;
        self
    }

    pub fn with_user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = user_agent.into();
        self
    }

    /// 修正非法取值，避免除零 / 死锁。
    pub(crate) fn normalized(&self) -> Self {
        let mut options = self.clone();
        options.max_connections = options.max_connections.max(1);
        options.max_active_files = options.max_active_files.max(1);
        options.max_workers = options.max_workers.max(1);
        options.chunk_size = options.chunk_size.max(1);
        options
    }

    /// 该大小的文件是否走分块下载。
    pub(crate) fn should_chunk(&self, size: u64) -> bool {
        size >= self.chunk_threshold && self.chunk_size > 0
    }

    /// 第 `retry` 次重试（从 1 开始）的指数退避等待时间。
    pub(crate) fn retry_delay(&self, retry: u32) -> Duration {
        let factor = 1u32 << retry.saturating_sub(1).min(31);
        self.retry_base_delay
            .saturating_mul(factor)
            .min(self.retry_max_delay)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunking_is_decided_by_size() {
        let options = DownloadOptions::default().with_chunking(1024, 256);
        assert!(!options.should_chunk(1023));
        assert!(options.should_chunk(1024));
        assert!(!options.without_chunking().should_chunk(u64::MAX - 1));
    }

    #[test]
    fn retry_delays_grow_exponentially_and_stop_at_the_cap() {
        let options = DownloadOptions::default()
            .with_retry_delay(Duration::from_millis(100), Duration::from_millis(500));
        assert_eq!(options.retry_delay(1), Duration::from_millis(100));
        assert_eq!(options.retry_delay(2), Duration::from_millis(200));
        assert_eq!(options.retry_delay(3), Duration::from_millis(400));
        assert_eq!(options.retry_delay(4), Duration::from_millis(500));
        assert_eq!(options.retry_delay(u32::MAX), Duration::from_millis(500));
    }

    #[test]
    fn illegal_values_are_normalised() {
        let options = DownloadOptions {
            max_connections: 0,
            max_active_files: 0,
            max_workers: 0,
            chunk_size: 0,
            ..DownloadOptions::default()
        }
        .normalized();
        assert_eq!(options.max_connections, 1);
        assert_eq!(options.max_active_files, 1);
        assert_eq!(options.max_workers, 1);
        assert_eq!(options.chunk_size, 1);
    }
}
