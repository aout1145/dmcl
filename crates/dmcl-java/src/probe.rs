//! 探测 java 可执行文件、解析版本横幅。

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use dmcl_mcmeta::os::Arch;

use crate::option::{JavaDetails, JavaOption};

pub(crate) async fn probe(path: PathBuf) -> Result<JavaDetails> {
    let mut command = tokio::process::Command::new(&path);
    command
        .arg("-XshowSettings:properties")
        .arg("-version")
        .kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(10), command.output())
        .await
        .map_err(|_| anyhow!("探测 {} 超时", path.display()))?
        .with_context(|| {
            format!(
                "无法执行 {} -XshowSettings:properties -version",
                path.display()
            )
        })?;

    // 属性与版本横幅通常都打在 stderr，有的实现写在 stdout，两个都收着
    let mut raw = String::from_utf8_lossy(&output.stderr).into_owned();
    raw.push_str(&String::from_utf8_lossy(&output.stdout));

    let property = |key: &str| {
        raw.lines()
            .find_map(|line| line.trim().strip_prefix(key))
            .unwrap_or_default()
            .trim()
            .to_string()
    };

    let version_str = {
        let property_version = property("java.version = ");
        if property_version.is_empty() {
            banner_version(&raw).unwrap_or_default()
        } else {
            property_version
        }
    };
    let major = major_version_of(&version_str).unwrap_or(0);
    let type_ = banner_type(&raw).unwrap_or("java").to_string();

    // 架构必须能从 `os.arch` 认出来：认不出就报错，让调用者跳过这个 java，
    // 而不是猜一个（natives 的 classifier 全押在它上面）
    let os_arch = property("os.arch = ");
    let arch = Arch::from_java_name(&os_arch).ok_or_else(|| {
        anyhow!(
            "无法确定 {} 的架构（os.arch = {os_arch:?}）",
            path.display()
        )
    })?;

    let java_home = property("java.home = ");
    let is_jdk = has_javac(&path, &java_home);

    Ok(JavaDetails {
        option: JavaOption { path, major, arch },
        type_,
        version_str,
        vendor: property("java.vendor = "),
        is_jdk,
    })
}

/// 版本横幅那一行：不缩进、且带着空格分隔的 `version`（属性行都缩进，
/// 首行是 `Property settings:`；`Picked up ...` 行不会命中）。
fn banner_line(raw: &str) -> Option<&str> {
    raw.lines().find(|line| {
        !line.starts_with(' ') && !line.starts_with('\t') && line.contains(" version ")
    })
}

/// 可执行文件旁边（或 `java.home` 的 bin 里）是否有 `javac`。
///
/// JDK 8 的 `java.home` 指向 `<jdk>/jre`，所以要同时看上一级的 `bin`。
fn has_javac(java: &Path, java_home: &str) -> bool {
    let javac = if cfg!(windows) { "javac.exe" } else { "javac" };
    if java.with_file_name(javac).is_file() {
        return true;
    }
    if java_home.is_empty() {
        return false;
    }
    let home = Path::new(java_home);
    home.join("bin").join(javac).is_file() || home.join("..").join("bin").join(javac).is_file()
}

fn banner_type(raw: &str) -> Option<&str> {
    banner_line(raw)?.split_whitespace().next()
}

fn banner_version(raw: &str) -> Option<String> {
    banner_line(raw)?
        .split_whitespace()
        .nth(2)
        .map(|value| value.trim_matches('"').to_string())
}

/// 从版本字符串取大版本号：`1.8.0_504` ⇒ 8，`17.0.2` ⇒ 17，`25-ea` ⇒ 25。
pub(crate) fn major_version_of(version: &str) -> Option<u32> {
    let mut parts = version.split('.');
    let first = leading_digits(parts.next()?)?;
    if first == 1 {
        leading_digits(parts.next()?)
    } else {
        Some(first)
    }
}

fn leading_digits(text: &str) -> Option<u32> {
    let digits: String = text.chars().take_while(|ch| ch.is_ascii_digit()).collect();
    if digits.is_empty() {
        None
    } else {
        digits.parse().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn major_version_handles_legacy_and_early_access_versions() {
        assert_eq!(major_version_of("1.8.0_504"), Some(8));
        assert_eq!(major_version_of("1.8"), Some(8));
        assert_eq!(major_version_of("17.0.2"), Some(17));
        assert_eq!(major_version_of("25-ea"), Some(25));
        assert_eq!(major_version_of(""), None);
        assert_eq!(major_version_of("abc"), None);
    }

    #[test]
    fn banner_parsing_ignores_tool_option_lines() {
        let raw = "Picked up JAVA_TOOL_OPTIONS: -Dfoo=version\n\
                   openjdk version \"17.0.2\" 2022-01-18\n\
                   OpenJDK Runtime Environment (build 17.0.2+8)\n";
        assert_eq!(banner_type(raw), Some("openjdk"));
        assert_eq!(banner_version(raw).as_deref(), Some("17.0.2"));
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn the_system_java_can_be_probed() {
        let Ok(java) = probe("/usr/bin/java".into()).await else {
            return; // 这台机器没有 java 就跳过
        };
        if java.is_usable() {
            assert!(matches!(
                java.option.arch,
                Arch::X86 | Arch::X86_64 | Arch::Aarch64
            ));
            assert!(!java.version_str.is_empty());
        }
    }
}
