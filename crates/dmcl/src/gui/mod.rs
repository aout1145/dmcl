//! 图形前端分派。
//!
//! 目前只有 GTK4（libadwaita）一个前端，实现放在 `gtk4` 子模块里、由 cargo feature
//! `gtk4` 门控。本模块**始终编译**，负责选前端并给出友好报错。

use anyhow::{anyhow, Result};

use crate::config::Config;

/// 不指定前端（`dmcl gui`）时用哪个。
pub fn default_frontend() -> &'static str {
    "gtk4"
}

/// 这份构建支持的前端（用于报错时提示）。
fn available_frontends() -> &'static [&'static str] {
    if cfg!(feature = "gtk4") {
        &["gtk4"]
    } else {
        &[]
    }
}

/// 启动图形前端，直到窗口关闭才返回。
pub fn run(frontend: Option<String>, config: Config) -> Result<()> {
    let frontend = frontend.unwrap_or_else(|| default_frontend().to_string());
    match frontend.as_str() {
        #[cfg(feature = "gtk4")]
        "gtk4" => gtk4::run(config),
        other => {
            // 未编译 gtk4 时明确报错（不回退到 prompt）。
            let _ = config;
            let available = available_frontends();
            let hint = if available.is_empty() {
                "；这份构建没有包含任何 gui 前端，请用 `cargo build --features gtk4` 重新构建"
            } else {
                ""
            };
            Err(anyhow!("没有可用的 gui 前端：{other}{hint}"))
        }
    }
}

#[cfg(feature = "gtk4")]
mod gtk4;
