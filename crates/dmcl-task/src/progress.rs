//! 进度快照。

use std::fmt;

/// 一个任务步骤的进度快照。
///
/// `current` / `total` 的单位由任务自己定义（字节数、文件个数……），前端只负责把它们
/// 渲染成字节数；**百分比只看 [`Progress::percent`]**：它由任务自己算好（有些任务的
/// 百分比不是 `current / total`），为 `None` 时表示这次进度没有有意义的百分比可显示。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Progress {
    /// 已完成的工作量（用于显示字节数）。
    pub current: u64,
    /// 总工作量；未知时为 `None`。
    pub total: Option<u64>,
    /// 面向用户的说明，例如当前正在下载的文件名。
    pub message: String,
    /// 有意义的百分比（0.0~100.0）；`None` 表示没有百分比可显示。
    pub percent: Option<f64>,
}

impl Progress {
    pub fn new(
        current: u64,
        total: Option<u64>,
        message: impl Into<String>,
        percent: Option<f64>,
    ) -> Self {
        Self {
            current,
            total,
            message: message.into(),
            percent,
        }
    }
}

impl fmt::Display for Progress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(percent) = self.percent {
            write!(
                f,
                "{percent:.1}% ({} / {})",
                human_bytes(self.current),
                human_bytes(self.total.unwrap_or(0))
            )?;
            if !self.message.is_empty() {
                write!(f, " {}", self.message)?;
            }
        } else if self.current > 0 {
            // 有字节数但总量未知
            write!(f, "({})", human_bytes(self.current))?;
            if !self.message.is_empty() {
                write!(f, " {}", self.message)?;
            }
        } else if self.message.is_empty() {
            write!(f, "…")?;
        } else {
            // 纯状态：只要文字，不带没意义的 "(0 B)"
            write!(f, "{}", self.message)?;
        }
        Ok(())
    }
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

    /// 百分比只由 `percent` 字段决定：没给就显示不出百分比。
    #[test]
    fn display_uses_the_percent_field() {
        // 没给百分比：只显示字节
        assert_eq!(
            Progress::new(512, Some(1024), "a.jar", None).to_string(),
            "(512 B) a.jar"
        );
        assert_eq!(
            Progress::new(512, Some(1024), "a.jar", Some(50.0)).to_string(),
            "50.0% (512 B / 1.0 KiB) a.jar"
        );
        // 起点：百分比 0 也照样显示成 0.0%
        assert_eq!(
            Progress::new(0, Some(0), "0/35 个文件", Some(0.0)).to_string(),
            "0.0% (0 B / 0 B) 0/35 个文件"
        );
    }

    #[test]
    fn display_without_a_percent_falls_back_to_bytes_or_message() {
        // 纯文字：不要出现 "(0 B)"
        assert_eq!(
            Progress::new(0, None, "scanning", None).to_string(),
            "scanning"
        );
        // 有字节数但总量未知
        assert_eq!(
            Progress::new(2048, None, "x", None).to_string(),
            "(2.0 KiB) x"
        );
        assert_eq!(Progress::new(0, None, "", None).to_string(), "…");
    }
}
