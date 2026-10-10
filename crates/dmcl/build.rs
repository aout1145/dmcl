//! 把构建时间写进环境变量，供 `dmcl version` 使用
//! —— 相当于 D 的 `__DATE__ __TIME__`。

use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    // 源码变化时重新生成时间戳（像 D 的 `__DATE__ __TIME__` 一样），
    // 而 `cargo build` 没有改动时仍然完全走缓存。
    println!("cargo:rerun-if-changed=src");

    let stamp = Command::new("date")
        .arg("+%Y-%m-%d %H:%M:%S")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|stamp| !stamp.is_empty())
        .unwrap_or_else(|| {
            let seconds = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_secs())
                .unwrap_or(0);
            format!("unix {seconds}")
        });

    println!("cargo:rustc-env=DMCL_BUILD_TIME={stamp}");
}
