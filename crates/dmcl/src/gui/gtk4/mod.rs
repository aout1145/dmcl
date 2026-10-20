#![doc = include_str!("README.md")]

pub(crate) mod app;
pub(crate) mod pages;

use crate::config::Config;

/// 启动 GTK4 界面；窗口关闭前一直阻塞在主线程。
pub fn run(config: Config) -> anyhow::Result<()> {
    app::run(config)
}
