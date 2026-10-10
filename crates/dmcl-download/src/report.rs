//! 下载结果汇总。

use std::path::PathBuf;

/// 一个失败的下载项。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadFailure {
    pub path: PathBuf,
    pub url: String,
    pub error: String,
}

impl std::fmt::Display for DownloadFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}]: {}", self.path.display(), self.error)
    }
}

/// 一次 [`crate::DownloadTask`] 运行的结果。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DownloadReport {
    /// 实际下载了多少个文件。
    pub downloaded: usize,
    /// 因为本地已是最新而跳过了多少个。
    pub skipped: usize,
    /// 失败项。
    pub failed: Vec<DownloadFailure>,
    /// 实际写入的字节数（不含跳过项）。
    pub bytes: u64,
}

impl DownloadReport {
    /// 是否全部成功。
    pub fn is_success(&self) -> bool {
        self.failed.is_empty()
    }

    /// 处理的文件总数。
    pub fn total(&self) -> usize {
        self.downloaded + self.skipped + self.failed.len()
    }

    /// 失败项的简要说明（用于错误信息）。
    pub fn failure_summary(&self, limit: usize) -> String {
        let mut lines: Vec<String> = self
            .failed
            .iter()
            .take(limit)
            .map(|failure| failure.to_string())
            .collect();
        if self.failed.len() > limit {
            lines.push(format!("……另有 {} 个失败项", self.failed.len() - limit));
        }
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_summarise_failures() {
        let report = DownloadReport {
            downloaded: 2,
            skipped: 1,
            failed: vec![DownloadFailure {
                path: PathBuf::from("/tmp/a.jar"),
                url: "https://example.com/a.jar".to_string(),
                error: "404".to_string(),
            }],
            bytes: 10,
        };
        assert!(!report.is_success());
        assert_eq!(report.total(), 4);
        assert!(report.failure_summary(5).contains("/tmp/a.jar"));

        assert!(DownloadReport::default().is_success());
        assert_eq!(DownloadReport::default().total(), 0);
    }
}
