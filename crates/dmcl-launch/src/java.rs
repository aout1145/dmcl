//! Java 探测、扫描与自动选择。
//!
//! 这里只处理**运行时的 Java 信息**：可执行文件路径、大版本、架构，以及探测得到的
//! 厂商 / 完整版本。配置表（名字 → 配置项）由调用者维护，通过
//! [`JavaSelector`] 注入选择结果；本模块不依赖 `IndexMap`。

use std::collections::BTreeMap;
use std::env;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use dmcl_mcmeta::os::Arch;
use dmcl_task::BoxFuture;
use walkdir::WalkDir;

/// 选择 Java 所需的最小信息：路径、大版本、架构。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JavaOption {
    /// java 可执行文件。
    pub path: PathBuf,
    /// 大版本号（`1.8` ⇒ 8，`17.0.2` ⇒ 17）。
    pub major: u32,
    /// JVM 架构（`os.arch`）。
    pub arch: Arch,
}

/// 探测得到的 Java 详细信息。
#[derive(Debug, Clone)]
pub struct JavaDetails {
    /// 选择 Java 所需的信息。
    pub option: JavaOption,
    /// 厂商 / 类型，`java -version` 输出的第一个词（例如 `openjdk`）。
    pub type_: String,
    /// 完整版本字符串（`17.0.2`）。
    pub version_str: String,
}

impl JavaDetails {
    /// 探测一个 java 可执行文件。
    ///
    /// 用 `-XshowSettings:properties -version`：一次调用既能拿到版本横幅，又能拿到
    /// `os.arch`（`java -version` 给不出 `x86_64` 这种信息）。
    pub async fn probe(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
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

        Ok(Self {
            option: JavaOption { path, major, arch },
            type_,
            version_str,
        })
    }

    /// 探测是否得到了可用的版本号。
    pub fn is_usable(&self) -> bool {
        self.option.major > 0
    }
}

/// 按版本要求提供 Java；由调用者（配置层）实现。
pub trait JavaSelector: Send + Sync {
    /// 返回满足 `required_major` 的 Java。
    fn select_java<'a>(&'a self, required_major: u32) -> BoxFuture<'a, Result<JavaOption>>;
}

/// 一个已经确定的 Java 可以直接当选择器用。
impl JavaSelector for JavaOption {
    fn select_java<'a>(&'a self, _required_major: u32) -> BoxFuture<'a, Result<JavaOption>> {
        Box::pin(async move { Ok(self.clone()) })
    }
}

/// 没有配置表时的默认选择器：扫描本机常见目录后自动挑选。
#[derive(Debug, Clone, Copy, Default)]
pub struct ScanJava;

impl JavaSelector for ScanJava {
    fn select_java<'a>(&'a self, required_major: u32) -> BoxFuture<'a, Result<JavaOption>> {
        Box::pin(async move {
            let details = find_java(&default_java_roots()).await;
            let options: Vec<JavaOption> = details.into_iter().map(|java| java.option).collect();
            auto_select_java(required_major, &options)
                .ok_or_else(|| anyhow!("未找到可用的 Java，请先安装 Java 或在配置中提供 Java 路径"))
        })
    }
}

/// 本机默认的 java 扫描目录（按平台）。
///
/// 返回的路径可能不存在；[`find_java`] 会跳过不存在的目录。
pub fn default_java_roots() -> Vec<PathBuf> {
    let mut roots = platform_java_roots();
    roots.sort();
    roots.dedup();
    roots
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
        roots.extend(VENDORS.iter().map(|vendor| base.join(vendor)));
        roots.extend(child_dirs_with_prefix(&base.join("Microsoft"), &["jdk"]));
    }
    if let Some(local) = env::var_os("LOCALAPPDATA").filter(|path| !path.is_empty()) {
        let programs = PathBuf::from(local).join("Programs");
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

/// 版本横幅那一行：不缩进、且带着空格分隔的 `version`（属性行都缩进，
/// 首行是 `Property settings:`；`Picked up ...` 行不会命中）。
fn banner_line(raw: &str) -> Option<&str> {
    raw.lines().find(|line| {
        !line.starts_with(' ') && !line.starts_with('\t') && line.contains(" version ")
    })
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
fn major_version_of(version: &str) -> Option<u32> {
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

/// 扫描目录里的 `java` 可执行文件，返回探测成功的详细信息。
///
/// **只返回结果，不写配置**；扫不到或全部探测失败时返回空列表。
pub async fn find_java(roots: &[impl AsRef<Path>]) -> Vec<JavaDetails> {
    let mut roots_to_scan: BTreeMap<PathBuf, PathBuf> = BTreeMap::new();
    for root in roots {
        let root = root.as_ref();
        let key = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        roots_to_scan
            .entry(key)
            .or_insert_with(|| root.to_path_buf());
    }

    let executable = if cfg!(windows) { "java.exe" } else { "java" };
    let mut candidates: BTreeMap<PathBuf, PathBuf> = BTreeMap::new();
    for root in roots_to_scan.values() {
        for entry in WalkDir::new(root)
            .into_iter()
            .filter_map(|entry| entry.ok())
        {
            if entry.file_name() != executable || !entry.path().is_file() {
                continue;
            }
            let canonical = entry
                .path()
                .canonicalize()
                .unwrap_or_else(|_| entry.path().to_path_buf());
            candidates
                .entry(canonical)
                .or_insert_with(|| entry.path().to_path_buf());
        }
    }

    let mut found = Vec::new();
    for path in candidates.values() {
        // 跳过探测失败或版本不可用的文件：一个野文件不该让整次搜索失败
        let Ok(java) = JavaDetails::probe(path.clone()).await else {
            continue;
        };
        if java.is_usable() {
            found.push(java);
        }
    }
    found
}

/// 从候选里挑最合适的 Java。
///
/// 只读缓存的 `major` / `arch`，不执行任何 Java。优先级：
/// 1. 大版本完全相等；
/// 2. 高于要求且最接近的；
/// 3. 低于要求且最高的；
///    同一档内优先当前平台架构，再按候选顺序。
pub fn auto_select_java(required_major: u32, options: &[JavaOption]) -> Option<JavaOption> {
    let host_arch = Arch::current();
    options
        .iter()
        .min_by_key(|java| {
            let (tier, distance) = match java.major.cmp(&required_major) {
                std::cmp::Ordering::Equal => (0, 0),
                std::cmp::Ordering::Greater => (1, java.major - required_major),
                std::cmp::Ordering::Less => (2, required_major - java.major),
            };
            let arch_mismatch = host_arch.is_some_and(|host| java.arch != host);
            (tier, distance, arch_mismatch)
        })
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn java(path: &str, major: u32) -> JavaOption {
        java_with_arch(path, major, Arch::X86_64)
    }

    fn java_with_arch(path: &str, major: u32, arch: Arch) -> JavaOption {
        JavaOption {
            path: PathBuf::from(path),
            major,
            arch,
        }
    }

    fn other_arch(host: Arch) -> Arch {
        match host {
            Arch::X86_64 => Arch::Aarch64,
            Arch::Aarch64 | Arch::X86 => Arch::X86_64,
        }
    }

    #[test]
    fn auto_selection_uses_cached_versions_without_running_java() {
        // 路径都不存在；能选中说明选择过程完全没有执行 java
        let options = vec![java("/jdk8", 8), java("/jdk17", 17), java("/jdk21", 21)];
        assert_eq!(auto_select_java(17, &options), Some(java("/jdk17", 17)));
        assert_eq!(auto_select_java(11, &options), Some(java("/jdk17", 17)));
        assert_eq!(auto_select_java(30, &options), Some(java("/jdk21", 21)));
        assert_eq!(auto_select_java(17, &[]), None);

        // 没有满足要求的版本时取最高的，不依赖配置顺序
        let lower = [java("/jdk8", 8), java("/jdk11", 11)];
        assert_eq!(auto_select_java(17, &lower), Some(java("/jdk11", 11)));
        let lower = [java("/jdk11", 11), java("/jdk8", 8)];
        assert_eq!(auto_select_java(17, &lower), Some(java("/jdk11", 11)));
    }

    #[test]
    fn auto_selection_prefers_the_current_architecture_within_a_version_tier() {
        let Some(host) = Arch::current() else {
            return;
        };
        let other = other_arch(host);
        let options = [
            java_with_arch("/other", 17, other),
            java_with_arch("/host", 17, host),
        ];
        assert_eq!(
            auto_select_java(17, &options),
            Some(java_with_arch("/host", 17, host))
        );
        assert_eq!(
            auto_select_java(11, &options),
            Some(java_with_arch("/host", 17, host))
        );
        assert_eq!(
            auto_select_java(21, &options),
            Some(java_with_arch("/host", 17, host))
        );
    }

    #[test]
    fn auto_selection_prefers_the_version_tier_over_the_architecture() {
        let Some(host) = Arch::current() else {
            return;
        };
        let other = other_arch(host);

        // 精确匹配的异架构版本优先于更高版本的当前架构版本
        let exact_other = [
            java_with_arch("/host-21", 21, host),
            java_with_arch("/other-17", 17, other),
        ];
        assert_eq!(
            auto_select_java(17, &exact_other),
            Some(java_with_arch("/other-17", 17, other))
        );

        // 高的兜底版本优先于低的当前架构版本
        let higher_other = [
            java_with_arch("/host-8", 8, host),
            java_with_arch("/other-11", 11, other),
        ];
        assert_eq!(
            auto_select_java(17, &higher_other),
            Some(java_with_arch("/other-11", 11, other))
        );
    }

    #[tokio::test]
    async fn a_java_option_is_a_fixed_selector() {
        let java = java("/jdk17", 17);
        assert_eq!(java.select_java(8).await.unwrap(), java);
    }

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

    #[test]
    fn child_dirs_with_prefix_only_returns_matching_directories() {
        let base = std::env::temp_dir().join(format!("dmcl-java-children-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        for name in ["openjdk", "openjdk@17", "zulu"] {
            std::fs::create_dir(base.join(name)).unwrap();
        }
        std::fs::write(base.join("openjdk-file"), b"").unwrap();

        let mut found = child_dirs_with_prefix(&base, &["openjdk"]);
        found.sort();
        assert_eq!(found, vec![base.join("openjdk"), base.join("openjdk@17")]);
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[tokio::test]
    async fn find_java_returns_an_empty_list_when_nothing_is_found() {
        let dir = std::env::temp_dir().join(format!("dmcl-java-empty-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();

        let found = find_java(&[dir.as_path()]).await;
        assert!(found.is_empty());
        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn find_java_accepts_a_symlinked_executable() {
        use std::os::unix::fs::{symlink, PermissionsExt};

        let root = std::env::temp_dir().join(format!("dmcl-java-symlink-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&root).await;
        tokio::fs::create_dir_all(&root).await.unwrap();

        let real = root.join("fake-java");
        tokio::fs::write(
            &real,
            "#!/bin/sh\ncat <<'EOF'\n\
             Property settings:\n\
             java.version = 17.0.8\n\
             os.arch = amd64\n\
             os.name = Linux\n\
             openjdk version \"17.0.8\" 2023-07-18\n\
             EOF\n",
        )
        .await
        .unwrap();
        let mut permissions = tokio::fs::metadata(&real).await.unwrap().permissions();
        permissions.set_mode(0o755);
        tokio::fs::set_permissions(&real, permissions)
            .await
            .unwrap();
        symlink(&real, root.join("java")).unwrap();

        let found = find_java(&[root.as_path()]).await;
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].option.path,
            root.join("java"),
            "符号链接本身应作为配置路径"
        );
        assert_eq!(found[0].type_, "openjdk");
        assert_eq!(found[0].option.major, 17);
        assert_eq!(found[0].option.arch, Arch::X86_64);

        tokio::fs::remove_dir_all(&root).await.unwrap();
    }

    #[test]
    fn default_java_roots_include_the_platform_directory() {
        let roots = default_java_roots();
        #[cfg(target_os = "linux")]
        assert!(roots.contains(&PathBuf::from("/usr/lib/jvm")));
        #[cfg(target_os = "macos")]
        assert!(roots.contains(&PathBuf::from("/Library/Java/JavaVirtualMachines")));
        #[cfg(target_os = "windows")]
        assert!(roots
            .iter()
            .any(|path| path.file_name().is_some_and(|name| name == "Java")));
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn the_system_java_can_be_probed() {
        let Ok(java) = JavaDetails::probe("/usr/bin/java").await else {
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
