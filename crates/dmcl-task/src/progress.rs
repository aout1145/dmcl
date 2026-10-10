//! 进度快照。

use std::fmt;

/// 一个任务步骤的进度快照。
///
/// `current` / `total` 的单位由任务自己定义（字节数、文件个数、步骤个数……），
/// 前端只负责把它们渲染成进度条或百分比。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Progress {
    /// 已完成的工作量。
    pub current: u64,
    /// 总工作量；服务端没有给出大小等情况下为 `None`。
    pub total: Option<u64>,
    /// 面向用户的说明，例如当前正在下载的文件名。
    pub message: String,
}

impl Progress {
    pub fn new(current: u64, total: Option<u64>, message: impl Into<String>) -> Self {
        Self {
            current,
            total,
            message: message.into(),
        }
    }

    /// 只有一条文字说明的进度（总量未知）。
    pub fn status(message: impl Into<String>) -> Self {
        Self {
            current: 0,
            total: None,
            message: message.into(),
        }
    }

    /// `current / total`，总量未知时为 `None`。
    pub fn fraction(&self) -> Option<f64> {
        match self.total {
            Some(0) | None => None,
            Some(total) => Some((self.current as f64 / total as f64).clamp(0.0, 1.0)),
        }
    }

    /// 百分比（0.0 ~ 100.0），总量未知时为 `None`。
    pub fn percent(&self) -> Option<f64> {
        self.fraction().map(|f| f * 100.0)
    }
}

impl fmt::Display for Progress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.percent(), self.total) {
            (Some(percent), Some(total)) => write!(
                f,
                "{percent:>5.1}% ({} / {})",
                human_bytes(self.current),
                human_bytes(total)
            )?,
            _ => write!(f, "({})", human_bytes(self.current))?,
        }
        if !self.message.is_empty() {
            write!(f, " {}", self.message)?;
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
    fn percent_and_fraction_are_clamped_and_total_less_is_none() {
        assert_eq!(Progress::new(1, Some(4), "").percent(), Some(25.0));
        assert_eq!(Progress::new(9, Some(4), "").percent(), Some(100.0));
        assert_eq!(Progress::new(9, None, "").percent(), None);
        assert_eq!(Progress::new(9, Some(0), "").percent(), None);
        assert_eq!(Progress::status("hi").fraction(), None);
    }

    #[test]
    fn bytes_are_formatted_in_binary_units() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(1023), "1023 B");
        assert_eq!(human_bytes(1024), "1.0 KiB");
        assert_eq!(human_bytes(1024 * 1024 * 3 / 2), "1.5 MiB");
    }

    #[test]
    fn display_combines_percent_and_message() {
        assert_eq!(
            Progress::new(512, Some(1024), "a.jar").to_string(),
            " 50.0% (512 B / 1.0 KiB) a.jar"
        );
        assert_eq!(Progress::status("scanning").to_string(), "(0 B) scanning");
    }
}
