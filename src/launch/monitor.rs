//! Game process monitor (was `dmcl.launch.monitor`).
//!
//! The original `GameMonitor` was an empty stub. Here it keeps the spawned
//! child so the caller can wait for the game to exit (and a GUI could later
//! stop it).

use anyhow::Result;

pub struct GameMonitor {
    child: tokio::process::Child,
}

impl GameMonitor {
    pub(crate) fn new(child: tokio::process::Child) -> Self {
        Self { child }
    }

    /// PID of the launched game, if still known.
    pub fn id(&self) -> Option<u32> {
        self.child.id()
    }

    /// Wait until the game exits.
    pub async fn wait(&mut self) -> Result<std::process::ExitStatus> {
        Ok(self.child.wait().await?)
    }
}
