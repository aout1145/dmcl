//! `version` —— 显示版本与构建时间。

use anyhow::Result;

use dmcl_launch::{LAUNCHER_NAME, LAUNCHER_VERSION};

/// `dmcl version`。
pub(crate) async fn run() -> Result<()> {
    let suffix = if cfg!(debug_assertions) {
        " (debug)"
    } else {
        ""
    };
    println!("{LAUNCHER_NAME} 版本 {LAUNCHER_VERSION}{suffix}");
    println!("构建时间：{}", env!("DMCL_BUILD_TIME"));
    println!();
    println!("感谢：bmclapi");
    Ok(())
}
