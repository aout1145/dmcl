//! Emits the build timestamp used by the `version` command — the Rust
//! equivalent of D's `__DATE__ __TIME__`.

use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    // Re-run whenever the sources change, so the stamp behaves like D's
    // `__DATE__ __TIME__` (a fresh time after every code change) while a no-op
    // `cargo build` stays fully cached.
    println!("cargo:rerun-if-changed=src");

    let stamp = Command::new("date")
        .arg("+%Y-%m-%d %H:%M:%S")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            let secs = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            format!("unix {secs}")
        });

    println!("cargo:rustc-env=DMCL_BUILD_TIME={stamp}");
}
