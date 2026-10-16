//! GTK4 + libadwaita 前端（基于 relm4）。
//!
//! 组件按「界面框架（`app`）」与「页面（`pages`）」粗分，详见仓库的 GUI 设计草稿。

pub(crate) mod app;
pub(crate) mod pages;

use crate::config::Config;

/// 启动 GTK4 界面；窗口关闭前一直阻塞在主线程。
pub fn run(config: Config) -> anyhow::Result<()> {
    app::run(config)
}
