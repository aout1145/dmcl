//! 进程退出方式：既用于 `launch.txt` 里的文字，也作为分析证据。

use std::fmt;
use std::process::ExitStatus;

use crate::Finding;

/// Windows 访问冲突（`0xC0000005`）对应的退出码。
const ACCESS_VIOLATION: i32 = -1073741819;

/// 游戏进程的退出方式。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Exit {
    /// 正常返回退出码。
    Code(i32),
    /// 被信号终止。
    Signal(i32),
    /// 既没有退出码也没有信号。
    Other,
    /// 连退出状态都没拿到。
    Unknown(String),
}

impl Exit {
    /// 从 `GameMonitor::wait()` 的结果构造。
    pub fn from_status(status: &Result<ExitStatus, String>) -> Self {
        match status {
            Ok(status) => match status.code() {
                Some(code) => Self::Code(code),
                None => {
                    #[cfg(unix)]
                    {
                        use std::os::unix::process::ExitStatusExt;
                        if let Some(signal) = status.signal() {
                            return Self::Signal(signal);
                        }
                    }
                    Self::Other
                }
            },
            Err(err) => Self::Unknown(err.clone()),
        }
    }

    /// 退出方式本身就能说明原因时的结论。
    pub(crate) fn finding(&self) -> Option<Finding> {
        let cause = match self {
            Self::Code(code) if *code == ACCESS_VIOLATION => {
                "游戏进程发生内存访问冲突（0xC0000005）"
            }
            Self::Code(_) => return None,
            Self::Signal(11) => "游戏进程发生段错误（SIGSEGV）",
            Self::Signal(6) => "游戏进程被主动中止（SIGABRT）",
            Self::Signal(_) => return None,
            Self::Other | Self::Unknown(_) => return None,
        };
        Some(Finding::new(
            cause,
            "多为原生库、显卡驱动或内存问题；查看 hs_err_pid*.log 或系统日志。",
        ))
    }
}

impl fmt::Display for Exit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Code(code) => write!(f, "退出码 {code}"),
            Self::Signal(signal) => write!(f, "被信号 {signal} 终止"),
            Self::Other => f.write_str("异常终止"),
            Self::Unknown(err) => write!(f, "无法获取退出状态：{err}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_kinds_are_described() {
        assert_eq!(Exit::Code(0).to_string(), "退出码 0");
        assert_eq!(Exit::Signal(9).to_string(), "被信号 9 终止");
        assert_eq!(Exit::Other.to_string(), "异常终止");
        assert_eq!(
            Exit::Unknown("等待失败".to_string()).to_string(),
            "无法获取退出状态：等待失败"
        );
    }

    #[test]
    fn abnormal_exit_kinds_point_at_native_crashes() {
        assert_eq!(
            Exit::Code(ACCESS_VIOLATION).finding().unwrap().cause,
            "游戏进程发生内存访问冲突（0xC0000005）"
        );
        assert_eq!(
            Exit::Signal(11).finding().unwrap().cause,
            "游戏进程发生段错误（SIGSEGV）"
        );
        assert!(Exit::Code(1).finding().is_none());
        assert!(Exit::Other.finding().is_none());
    }
}
