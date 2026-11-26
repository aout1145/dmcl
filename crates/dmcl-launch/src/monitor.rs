//! 游戏进程监控。

use std::fmt;

use anyhow::{anyhow, Result};

use crate::option::ProcessPriority;

/// 被启动的游戏进程。
///
/// 原来是空 stub；这里持有子进程句柄，因此调用者可以等它退出，
/// 也可以主动把它停掉（GUI 的"停止游戏"）。
pub struct GameMonitor {
    child: tokio::process::Child,
}

impl fmt::Debug for GameMonitor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GameMonitor")
            .field("pid", &self.id())
            .finish()
    }
}

impl GameMonitor {
    pub(crate) fn new(child: tokio::process::Child) -> Self {
        Self { child }
    }

    /// 游戏进程的 PID（如果还知道）。
    pub fn id(&self) -> Option<u32> {
        self.child.id()
    }

    /// 游戏是否已经退出（不阻塞）。
    pub fn try_status(&mut self) -> Result<Option<std::process::ExitStatus>> {
        Ok(self.child.try_wait()?)
    }

    /// 结束游戏进程。
    pub async fn kill(&mut self) -> Result<()> {
        self.child.kill().await?;
        Ok(())
    }

    /// 等到游戏退出。
    pub async fn wait(&mut self) -> Result<std::process::ExitStatus> {
        Ok(self.child.wait().await?)
    }
}

/// 用给定的工作目录与命令行参数启动游戏子进程，返回**未 wait** 的 [`GameMonitor`]。
///
/// 抽成自由函数是为了让 [`crate::LaunchTask`] 能直接用 [`crate::PreparedLaunch`]
/// 里已有的参数启动，不必再建一个 [`crate::GameLauncher`]、再选一次 java。这里**不打印**
/// 任何东西：命令行由 [`crate::LaunchTask`] 通过进度事件流展示。
pub(crate) fn spawn_game(
    game_path: &std::path::Path,
    args: &[String],
    priority: ProcessPriority,
) -> Result<GameMonitor> {
    if args.is_empty() {
        return Err(anyhow!("命令为空"));
    }
    let mut command = tokio::process::Command::new(&args[0]);
    command.args(&args[1..]).current_dir(game_path);
    apply_priority(&mut command, priority);
    let child = command.spawn()?;
    Ok(GameMonitor::new(child))
}

#[cfg(windows)]
fn apply_priority(command: &mut tokio::process::Command, priority: ProcessPriority) {
    // `CreateProcess` 的进程优先级类；正常优先级不需要额外标志。
    const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x0000_4000;
    const ABOVE_NORMAL_PRIORITY_CLASS: u32 = 0x0000_8000;

    let flags = match priority {
        ProcessPriority::Low => BELOW_NORMAL_PRIORITY_CLASS,
        ProcessPriority::Normal => 0,
        ProcessPriority::High => ABOVE_NORMAL_PRIORITY_CLASS,
    };
    if flags != 0 {
        command.creation_flags(flags);
    }
}

#[cfg(unix)]
fn apply_priority(command: &mut tokio::process::Command, priority: ProcessPriority) {
    let nice = match priority {
        ProcessPriority::Low => 10,
        ProcessPriority::Normal => 0,
        ProcessPriority::High => -5,
    };
    if nice == 0 {
        return;
    }
    // 负 nice 需要权限；没有权限时保持当前优先级启动，而不是让游戏起不来。
    unsafe {
        command.pre_exec(move || {
            if libc::setpriority(libc::PRIO_PROCESS, 0, nice) == 0 {
                return Ok(());
            }
            let error = std::io::Error::last_os_error();
            match error.raw_os_error() {
                Some(libc::EPERM) | Some(libc::EACCES) => Ok(()),
                _ => Err(error),
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一个「睡很久」的命令 —— 两个平台都有。
    fn long_running() -> tokio::process::Command {
        #[cfg(windows)]
        {
            // Windows 没有 `sleep`，用 `ping` 当等待；`>NUL` 丢掉输出
            let mut command = tokio::process::Command::new("cmd");
            command.args(["/C", "ping -n 30 127.0.0.1 >NUL"]);
            command
        }
        #[cfg(not(windows))]
        {
            let mut command = tokio::process::Command::new("sleep");
            command.arg("30");
            command
        }
    }

    #[tokio::test]
    async fn a_monitor_reports_exit_and_can_kill() {
        let child = long_running().spawn().expect("spawn long-running command");
        let mut monitor = GameMonitor::new(child);
        assert!(monitor.id().is_some());
        assert!(monitor.try_status().unwrap().is_none());
        monitor.kill().await.unwrap();
        // 被强杀：unix 死于信号（没有退出码），windows 是一个非 0 退出码
        assert!(!monitor.wait().await.unwrap().success());
    }
}
