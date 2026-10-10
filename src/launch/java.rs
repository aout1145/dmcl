//! Java discovery and selection (was `dmcl.launch.java`).

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use walkdir::WalkDir;

use crate::config::{save_config, with_config, with_config_mut, JavaChoice};

/// `JavaOption` — probe a java executable with `-version`.
#[derive(Debug, Clone)]
pub struct JavaOption {
    /// The java executable.
    pub path: PathBuf,
    /// Vendor/type, first token of `java -version` output (e.g. `openjdk`).
    pub type_: String,
    /// 32 or 64 — substituted into `${arch}`.
    pub arch: u32,
    /// Major version (`1.8` ⇒ 8, `17.0.2` ⇒ 17) — never negative.
    pub major_version: u32,
    /// Full version string (`17.0.2`).
    pub version_str: String,
}

impl JavaOption {
    pub async fn new(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let output = tokio::process::Command::new(&path)
            .arg("-version")
            .output()
            .await
            .with_context(|| format!("failed to run {} -version", path.display()))?;

        // `java -version` prints on stderr, the original `std.process.execute`
        // collected it as well; keep both for safety.
        let mut raw = String::from_utf8_lossy(&output.stderr).into_owned();
        raw.push_str(&String::from_utf8_lossy(&output.stdout));
        let raw = raw.replace('"', "").replace('\n', " ");

        let tokens: Vec<&str> = raw.split(' ').collect();
        let type_ = tokens.first().copied().unwrap_or("").to_string();
        let version_str = tokens.get(2).copied().unwrap_or("").to_string();
        let arch = if raw.contains("64-Bit") { 64 } else { 32 };
        let vers: Vec<&str> = version_str.split('.').collect();
        let major_version = if vers.first().copied().unwrap_or("") != "1" {
            vers.first()
                .and_then(|v| v.parse::<u32>().ok())
                .unwrap_or(0)
        } else {
            vers.get(1).and_then(|v| v.parse::<u32>().ok()).unwrap_or(0)
        };

        Ok(Self {
            path,
            type_,
            arch,
            major_version,
            version_str,
        })
    }
}

/// `findJava` — scan `/lib/jvm` (posix) for java executables and store them in
/// the configuration.
pub async fn find_java() -> Result<()> {
    if cfg!(target_os = "windows") {
        return Err(anyhow!("finding java are not supported on windows!"));
    }
    if !cfg!(unix) {
        return Err(anyhow!("finding java are not supported on this platform!"));
    }
    let possible_paths = ["/lib/jvm/"];

    println!("find following javas:");
    let mut java_configs: BTreeMap<String, PathBuf> = BTreeMap::new();
    for base in possible_paths {
        for entry in WalkDir::new(base).into_iter().filter_map(|e| e.ok()) {
            if !entry.file_type().is_file() {
                continue;
            }
            if entry.file_name() != "java" {
                continue;
            }
            let path = entry.path().to_path_buf();
            // The original aborts on a broken executable; skipping it is a
            // deliberate robustness improvement (a stray `java` file must not
            // kill the whole scan).
            match JavaOption::new(path).await {
                Ok(java) if java.major_version > 0 => {
                    let name = format!("{}-{}-bit-{}", java.type_, java.arch, java.version_str);
                    println!("{name}: {}", java.path.display());
                    java_configs.insert(name, java.path);
                }
                _ => continue,
            }
        }
    }
    with_config_mut(|c| {
        c.launch.java_configs = java_configs;
        c.launch.java = JavaChoice::Auto;
    });
    save_config().await?;
    Ok(())
}

/// `selectJava` — resolve the configured java, or auto-select the closest match
/// for the required major version.
pub async fn select_java(required_major: u32) -> Result<JavaOption> {
    let java_name = match with_config(|c| c.launch.java.clone()) {
        JavaChoice::Named(name) => name,
        JavaChoice::Auto => auto_select_java(required_major).await?,
    };

    let path = with_config(|c| c.launch.java_configs.get(&java_name).cloned())
        .ok_or_else(|| anyhow!("no such configured java: {java_name}"))?;
    let java = JavaOption::new(path).await?;
    if java.major_version != required_major {
        println!(
            "warning: using unsupported java version(require {required_major} but using {})",
            java.major_version
        );
    }
    Ok(java)
}

/// The `autoselect` search: an exact major-version match wins, otherwise the
/// closest java that is at least `required_major`, otherwise the first
/// configured entry.
async fn auto_select_java(required_major: u32) -> Result<String> {
    if with_config(|c| c.launch.java_configs.is_empty()) {
        find_java().await?;
    }
    let configs = with_config(|c| c.launch.java_configs.clone());
    if configs.is_empty() {
        return Err(anyhow!("no configured java"));
    }

    let mut best: Option<String> = None;
    let mut best_diff = u32::MAX;
    for (name, path) in &configs {
        let thisjava = JavaOption::new(path.clone()).await?;
        if thisjava.major_version == required_major {
            return Ok(name.clone());
        }
        // `major_version` is unsigned, so no `abs()` is needed here
        let diff = thisjava.major_version.saturating_sub(required_major);
        if thisjava.major_version >= required_major && diff < best_diff {
            best_diff = diff;
            best = Some(name.clone());
        }
    }
    Ok(best.unwrap_or_else(|| configs.keys().next().cloned().unwrap_or_default()))
}
