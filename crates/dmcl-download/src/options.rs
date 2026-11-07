//! 下载的并发与分块策略。

/// 默认的 User-Agent（与其它 crate 保持一致，但各自独立，避免相互依赖）。
pub const DEFAULT_USER_AGENT: &str = concat!("dmcl/", env!("CARGO_PKG_VERSION"));

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
