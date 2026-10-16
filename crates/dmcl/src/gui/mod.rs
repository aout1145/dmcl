//! 图形前端分派。
//!
//! 目前只有 GTK4（libadwaita）一个前端，实现放在 `gtk4` 子模块里，由 cargo feature
//! `gtk4` 门控（默认关，用 `--features gtk4` 开启）。本模块始终编译，负责选前端并给友好报错。

use anyhow::{anyhow, Result};

use crate::config::Config;

/// 不指定前端（`dmcl gui`）时用哪个。
pub fn default_frontend() -> &'static str {
    "gtk4"
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
            let hint = if cfg!(feature = "gtk4") {
                ""
            } else {
                "；这份构建没有包含 gtk4 前端（用 `--features gtk4` 重新构建）"
            };
            Err(anyhow!("没有可用的 gui 前端：{other}{hint}"))
        }
    }
}

#[cfg(feature = "gtk4")]
mod gtk4;
