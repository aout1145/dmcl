//! 图形前端分派。
//!
//! 目前只有 GTK4（libadwaita）一个前端，实现放在 `gtk4` 子模块里，由 cargo feature
//! `gtk4` 门控（默认关，用 `--features gtk4` 开启）。本模块始终编译，负责选前端；
//! 没有可用的前端时回退到交互式菜单。

use anyhow::Result;

use crate::config::Config;

/// 不指定前端（`dmcl gui`）时用哪个。
pub fn default_frontend() -> &'static str {
    "gtk4"
}

/// 启动图形前端，直到窗口关闭才返回。
///
/// 指定的前端不可用（最常见的是这份构建没编译 `gtk4`）时，回退到交互式菜单。
pub fn run(frontend: Option<String>, mut config: Config) -> Result<()> {
    let frontend = frontend.unwrap_or_else(|| default_frontend().to_string());
    match frontend.as_str() {
        #[cfg(feature = "gtk4")]
        "gtk4" => gtk4::run(config),
        _ => crate::cli::run_prompt(&mut config),
    }
}

#[cfg(feature = "gtk4")]
mod gtk4;
