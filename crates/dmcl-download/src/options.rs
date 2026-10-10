//! 下载的并发与分块策略。

/// 默认的 User-Agent（与其它 crate 保持一致，但各自独立，避免相互依赖）。
pub const DEFAULT_USER_AGENT: &str = concat!("dmcl/", env!("CARGO_PKG_VERSION"));

/// 下载的并发与分块策略。
///
/// 全部由调用者提供 —— 这个 crate 不读任何配置文件。
#[derive(Debug, Clone)]
pub struct DownloadOptions {
    /// 同时进行的连接数上限。
    pub max_connections: usize,
    /// 超过这个字节数就分块下载。
    pub chunk_threshold: u64,
    /// 每块的大小。
    pub chunk_size: u64,
    /// 单个文件最多并发几块（防止一个大文件把连接数吃光）。
    pub max_chunks_per_file: usize,
    /// 单个文件最多尝试几次（含第一次）。
    pub max_attempts: u32,
    /// 单个文件失败后是否继续下载其余文件。
    ///
    /// `false`（默认）时，只要有一个文件失败，任务最后会返回错误；
    /// 无论哪种取值，所有已排队的文件都会跑完，报告里能看到全部失败项。
    pub ignore_failures: bool,
    pub user_agent: String,
}

impl Default for DownloadOptions {
    fn default() -> Self {
        Self {
            max_connections: 64,
            chunk_threshold: 8 * 1024 * 1024,
            chunk_size: 4 * 1024 * 1024,
            max_chunks_per_file: 8,
            max_attempts: 3,
            ignore_failures: false,
            user_agent: DEFAULT_USER_AGENT.to_string(),
        }
    }
}

impl DownloadOptions {
    /// 只要并发数、其余取默认值。
    pub fn with_max_connections(max_connections: usize) -> Self {
        Self {
            max_connections: max_connections.max(1),
            ..Self::default()
        }
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

    pub fn with_max_attempts(mut self, max_attempts: u32) -> Self {
        self.max_attempts = max_attempts.max(1);
        self
    }

    pub fn with_user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = user_agent.into();
        self
    }

    pub fn with_ignore_failures(mut self, ignore_failures: bool) -> Self {
        self.ignore_failures = ignore_failures;
        self
    }

    /// 修正非法取值，避免除零 / 死锁。
    pub(crate) fn normalized(&self) -> Self {
        let mut options = self.clone();
        options.max_connections = options.max_connections.max(1);
        options.chunk_size = options.chunk_size.max(1);
        options.max_chunks_per_file = options.max_chunks_per_file.max(1);
        options.max_attempts = options.max_attempts.max(1);
        options
    }

    /// 该大小的文件是否走分块下载。
    pub(crate) fn should_chunk(&self, size: u64) -> bool {
        size >= self.chunk_threshold && self.chunk_size > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sane() {
        let options = DownloadOptions::default();
        assert!(options.max_connections >= 1);
        assert!(options.chunk_size >= 1);
        assert!(options.max_attempts >= 1);
        assert!(!options.ignore_failures);
        assert_eq!(DownloadOptions::with_max_connections(0).max_connections, 1);
        assert_eq!(options.clone().with_max_attempts(0).max_attempts, 1);
        assert_eq!(options.clone().without_chunking().chunk_threshold, u64::MAX);
        assert!(options.with_ignore_failures(true).ignore_failures);
    }

    #[test]
    fn chunking_is_decided_by_size() {
        let options = DownloadOptions::default().with_chunking(1024, 256);
        assert!(!options.should_chunk(1023));
        assert!(options.should_chunk(1024));
        assert!(!options.without_chunking().should_chunk(u64::MAX - 1));
    }
}
