//! 图形前端分派。
//!
//! 目前只有 GTK4（libadwaita）一个前端，实现放在 `gtk4` 子模块里，由 cargo feature
//! `gtk4` 门控（默认关，用 `--features gtk4` 开启）。显式的 `gui` 命令在前端不可用时
//! 直接报错；无子命令走 [`run_default`]（有前端就启动 GUI，否则进交互菜单）。

use anyhow::{anyhow, Result};

use crate::config::Config;

/// 不指定前端（`dmcl gui`）时用哪个。
pub fn default_frontend() -> &'static str {
    "gtk4"
}

/// 指定的前端是否编译进了这份构建。
pub fn is_available(frontend: Option<&str>) -> bool {
    let frontend = frontend.unwrap_or(default_frontend());
    match frontend {
        #[cfg(feature = "gtk4")]
        "gtk4" => true,
        _ => false,
    }
}

/// 无子命令时的默认路径：有 GUI 前端就启动 GUI，否则进交互菜单。
pub fn run_default(mut config: Config) -> Result<()> {
    if is_available(None) {
        run(None, config)
    } else {
        crate::cli::run_prompt(&mut config)
    }
}

/// 启动图形前端，直到窗口关闭才返回。
///
/// 前端不可用时**直接报错**，不回退到交互式菜单；回退只发生在无子命令的默认路径
/// [`run_default`] 上。
pub fn run(frontend: Option<String>, config: Config) -> Result<()> {
    // 没编译 gtk4 时 config 不会进入 gtk4::run，显式消费掉避免 unused 警告。
    #[cfg(not(feature = "gtk4"))]
    let _ = config;
    let frontend = frontend.unwrap_or_else(|| default_frontend().to_string());
    match frontend.as_str() {
        #[cfg(feature = "gtk4")]
        "gtk4" => gtk4::run(config),
        other => {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn availability_matches_the_compiled_feature() {
        assert_eq!(is_available(None), cfg!(feature = "gtk4"));
        assert_eq!(is_available(Some("gtk4")), cfg!(feature = "gtk4"));
        assert!(!is_available(Some("other")));
    }

    #[test]
    fn unknown_frontend_is_an_error() {
        let err = run(Some("other".to_string()), Config::default()).unwrap_err();
        assert!(err.to_string().contains("没有可用的 gui 前端"));
    }
}
