//! 全局下载进度。

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

/// 整个任务图共享的下载进度。
#[derive(Clone)]
pub struct ProgressHandle {
    state: Arc<ProgressState>,
}

struct ProgressState {
    downloaded: AtomicI64,
    total: AtomicI64,
}

impl Default for ProgressHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl ProgressHandle {
    pub fn new() -> Self {
        Self {
            state: Arc::new(ProgressState {
                downloaded: AtomicI64::new(0),
                total: AtomicI64::new(0),
            }),
        }
    }

    /// 调整总字节数。正数增加，负数减少（服务器实际大小更小、失败回滚等）。
    pub fn add_total(&self, delta: i64) {
        add_saturating(&self.state.total, delta);
    }

    /// 调整已下载字节。正数增加，负数扣回（重试前撤销本轮已报告字节等）。
    pub fn add_downloaded(&self, delta: i64) {
        add_saturating(&self.state.downloaded, delta);
    }

    /// 当前快照；负值被 clamp 到 0。
    pub fn snapshot(&self) -> ProgressSnapshot {
        ProgressSnapshot {
            downloaded: clamp_u64(self.state.downloaded.load(Ordering::SeqCst)),
            total: clamp_u64(self.state.total.load(Ordering::SeqCst)),
        }
    }
}

fn add_saturating(value: &AtomicI64, delta: i64) {
    let _ = value.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
        Some(current.saturating_add(delta))
    });
}

fn clamp_u64(value: i64) -> u64 {
    value.max(0) as u64
}

/// 全局进度的只读快照。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ProgressSnapshot {
    pub downloaded: u64,
    pub total: u64,
}

/// 把字节数渲染成人类可读的形式（1024 进制）。
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_are_formatted_in_binary_units() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(1023), "1023 B");
        assert_eq!(human_bytes(1024), "1.0 KiB");
        assert_eq!(human_bytes(1024 * 1024 * 3 / 2), "1.5 MiB");
    }

    #[test]
    fn totals_and_downloaded_can_increase_and_decrease() {
        let progress = ProgressHandle::new();
        progress.add_total(100);
        progress.add_downloaded(40);
        assert_eq!(
            progress.snapshot(),
            ProgressSnapshot {
                downloaded: 40,
                total: 100
            }
        );

        // 服务器实际更小：总大小减少。
        progress.add_total(-30);
        // 重试：扣回已报告字节。
        progress.add_downloaded(-25);
        assert_eq!(
            progress.snapshot(),
            ProgressSnapshot {
                downloaded: 15,
                total: 70
            }
        );
    }

    #[test]
    fn snapshot_never_reports_negative_values() {
        let progress = ProgressHandle::new();
        progress.add_total(-10);
        progress.add_downloaded(-20);
        assert_eq!(progress.snapshot(), ProgressSnapshot::default());
    }

    #[test]
    fn arithmetic_saturates_instead_of_overflowing() {
        let progress = ProgressHandle::new();
        progress.add_total(i64::MAX);
        progress.add_total(i64::MAX);
        assert_eq!(progress.snapshot().total, i64::MAX as u64);

        progress.add_downloaded(i64::MIN);
        progress.add_downloaded(i64::MIN);
        assert_eq!(progress.snapshot().downloaded, 0);
    }
}
