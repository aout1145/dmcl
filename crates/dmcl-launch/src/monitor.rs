//! 游戏进程监控。

use anyhow::Result;

/// 被启动的游戏进程。
///
/// 原来是空 stub；这里持有子进程句柄，因此调用者可以等它退出，
/// 也可以主动把它停掉（GUI 的"停止游戏"）。
pub struct GameMonitor {
    child: tokio::process::Child,
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

    #[tokio::test]
    async fn a_monitor_reports_exit_and_can_kill() {
        let child = tokio::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn sleep");
        let mut monitor = GameMonitor::new(child);
        assert!(monitor.id().is_some());
        assert!(monitor.try_status().unwrap().is_none());
        monitor.kill().await.unwrap();
        assert!(monitor.wait().await.unwrap().code().is_none());
    }
}
