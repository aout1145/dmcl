//! 本机常见的 Java 安装目录与官方启动器的 runtime 目录。

use std::env;
use std::path::{Path, PathBuf};

/// 本机默认的 java 扫描目录（按平台）。
///
/// 返回的路径可能不存在；[`crate::find_java`] 会跳过不存在的目录。
pub fn default_java_roots() -> Vec<PathBuf> {
    let mut roots = platform_java_roots();
    roots.sort();
    roots.dedup();
    roots
}

/// 官方启动器存放 Java 运行时的目录。
///
/// 自动下载的 JRE 也装在这里；平台环境变量缺失时返回 `None`。
pub fn official_runtime_root() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        let base = env::var_os("APPDATA").filter(|path| !path.is_empty())?;
        Some(PathBuf::from(base).join(".minecraft").join("runtime"))
    }
    #[cfg(target_os = "macos")]
    {
        let base = env::var_os("HOME").filter(|path| !path.is_empty())?;
        Some(PathBuf::from(base).join("Library/Application Support/minecraft/runtime"))
    }
    #[cfg(target_os = "linux")]
    {
        let base = env::var_os("HOME").filter(|path| !path.is_empty())?;
        Some(PathBuf::from(base).join(".minecraft").join("runtime"))
    }
}

#[cfg(target_os = "linux")]
fn platform_java_roots() -> Vec<PathBuf> {
    let mut roots = vec![
        PathBuf::from("/lib/jvm"),
        PathBuf::from("/usr/lib/jvm"),
        PathBuf::from("/usr/lib64/jvm"),
        PathBuf::from("/usr/java"),
        PathBuf::from("/opt/java"),
        PathBuf::from("/opt/jdk"),
        PathBuf::from("/usr/local/java"),
    ];
    if let Some(home) = env::var_os("HOME").filter(|home| !home.is_empty()) {
        let home = PathBuf::from(home);
        roots.push(home.join(".sdkman/candidates/java"));
        roots.push(home.join(".jdks"));
        roots.push(home.join(".gradle/jdks"));
        roots.push(home.join(".asdf/installs/java"));
        roots.push(home.join(".local/share/mise/installs/java"));
        roots.extend(child_dirs_with_prefix(
            &home.join(".linuxbrew/opt"),
            &["openjdk"],
        ));
    }
    roots.extend(official_runtime_root());
    roots.extend(child_dirs_with_prefix(
        Path::new("/home/linuxbrew/.linuxbrew/opt"),
        &["openjdk"],
    ));
    if let Some(java_home) = env::var_os("JAVA_HOME").filter(|home| !home.is_empty()) {
        roots.push(PathBuf::from(java_home).join("bin"));
    }
    roots
}

#[cfg(target_os = "windows")]
fn platform_java_roots() -> Vec<PathBuf> {
    const VENDORS: &[&str] = &[
        "Java",
        "Eclipse Adoptium",
        "Eclipse Foundation",
        "AdoptOpenJDK",
        "Zulu",
        "BellSoft",
        "Amazon Corretto",
        "Semeru",
        "SapMachine",
        "GraalVM",
    ];

    let mut roots = Vec::new();
    for var in ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"] {
        let Some(base) = env::var_os(var).filter(|path| !path.is_empty()) else {
            continue;
        };
        let base = PathBuf::from(base);
        roots.push(base.join("Minecraft Launcher").join("runtime"));
        roots.extend(VENDORS.iter().map(|vendor| base.join(vendor)));
        roots.extend(child_dirs_with_prefix(&base.join("Microsoft"), &["jdk"]));
    }
    if let Some(local) = env::var_os("LOCALAPPDATA").filter(|path| !path.is_empty()) {
        let local = PathBuf::from(local);
        // Microsoft Store 版官方启动器的 LocalCache。
        roots.push(
            local
                .join("Packages")
                .join("Microsoft.4297127D64EC6_8wekyb3d8bbwe")
                .join("LocalCache")
                .join("Local")
                .join("runtime"),
        );
        let programs = local.join("Programs");
        roots.push(programs.join("Eclipse Adoptium"));
        roots.push(programs.join("Java"));
        roots.extend(child_dirs_with_prefix(
            &programs.join("Microsoft"),
            &["jdk"],
        ));
    }
    if let Some(profile) = env::var_os("USERPROFILE").filter(|path| !path.is_empty()) {
        roots.push(PathBuf::from(profile).join(".jdks"));
    }
    roots.extend(official_runtime_root());
    if let Some(java_home) = env::var_os("JAVA_HOME").filter(|home| !home.is_empty()) {
        roots.push(PathBuf::from(java_home).join("bin"));
    }
    roots
}

#[cfg(target_os = "macos")]
fn platform_java_roots() -> Vec<PathBuf> {
    let mut roots = vec![
        PathBuf::from("/Library/Java/JavaVirtualMachines"),
        PathBuf::from("/System/Library/Java/JavaVirtualMachines"),
    ];
    if let Some(home) = env::var_os("HOME").filter(|home| !home.is_empty()) {
        let home = PathBuf::from(home);
        roots.push(home.join("Library/Java/JavaVirtualMachines"));
        roots.push(home.join(".sdkman/candidates/java"));
    }
    roots.extend(official_runtime_root());
    roots.extend(child_dirs_with_prefix(
        Path::new("/opt/homebrew/opt"),
        &["openjdk"],
    ));
    roots.extend(child_dirs_with_prefix(
        Path::new("/usr/local/opt"),
        &["openjdk"],
    ));
    if let Some(java_home) = env::var_os("JAVA_HOME").filter(|home| !home.is_empty()) {
        roots.push(PathBuf::from(java_home).join("bin"));
    }
    roots
}

/// 列出目录下名字以指定前缀开头的子目录（跟随指向目录的符号链接）。
fn child_dirs_with_prefix(base: &Path, prefixes: &[&str]) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(base) else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            prefixes.iter().any(|prefix| name.starts_with(prefix))
        })
        .filter(|entry| entry.path().is_dir())
        .map(|entry| entry.path())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("dmcl-java-roots-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn child_dirs_with_prefix_only_returns_matching_directories() {
        let base = temp_dir("children");
        for name in ["openjdk", "openjdk@17", "zulu"] {
            std::fs::create_dir(base.join(name)).unwrap();
        }
        std::fs::write(base.join("openjdk-file"), b"").unwrap();

        let mut found = child_dirs_with_prefix(&base, &["openjdk"]);
        found.sort();
        assert_eq!(found, vec![base.join("openjdk"), base.join("openjdk@17")]);
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn default_java_roots_include_the_platform_directory() {
        let roots = default_java_roots();
        #[cfg(target_os = "linux")]
        {
            assert!(roots.contains(&PathBuf::from("/usr/lib/jvm")));
            if let Some(home) = env::var_os("HOME").filter(|home| !home.is_empty()) {
                assert!(roots.contains(&PathBuf::from(home).join(".minecraft/runtime")));
            }
        }
        #[cfg(target_os = "macos")]
        {
            assert!(roots.contains(&PathBuf::from("/Library/Java/JavaVirtualMachines")));
            if let Some(home) = env::var_os("HOME").filter(|home| !home.is_empty()) {
                assert!(roots.contains(
                    &PathBuf::from(home).join("Library/Application Support/minecraft/runtime")
                ));
            }
        }
        #[cfg(target_os = "windows")]
        {
            assert!(roots
                .iter()
                .any(|path| path.file_name().is_some_and(|name| name == "Java")));
            assert!(roots
                .iter()
                .any(|path| path.ends_with("Minecraft Launcher/runtime")));
            if let Some(appdata) = env::var_os("APPDATA").filter(|path| !path.is_empty()) {
                assert!(roots.contains(&PathBuf::from(appdata).join(".minecraft").join("runtime")));
            }
        }
    }
}
