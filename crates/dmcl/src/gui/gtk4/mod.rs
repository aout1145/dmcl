#![doc = include_str!("README.md")]

pub(crate) mod app;
pub(crate) mod pages;

use crate::config::LoadedConfig;

/// 启动 GTK4 界面；窗口关闭前一直阻塞在主线程。
pub fn run(loaded: LoadedConfig) -> anyhow::Result<()> {
    app::run(loaded)
}
