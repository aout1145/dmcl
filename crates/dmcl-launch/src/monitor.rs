//! 游戏进程监控。

use std::fmt;

use anyhow::Result;

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
