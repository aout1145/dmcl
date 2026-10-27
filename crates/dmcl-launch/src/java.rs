//! Java 探测与选择。
//!
//! **这里不碰配置**：配置表由调用者传入 / 接收（见 [`JavaConfigs`]）。

use std::collections::BTreeMap;
use std::env;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use dmcl_mcmeta::os::{Arch, Os};
use indexmap::IndexMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha1::{Digest, Sha1};
use walkdir::WalkDir;

/// java 配置表：名字 → 可执行文件，保持顺序。
pub type JavaConfigs = IndexMap<String, PathBuf>;

/// 表示「自动选择」的哨兵字符串。
pub const JAVA_AUTOSELECT: &str = "autoselect";

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

/// `launch.java` —— 要么自动选择，要么是 [`JavaConfigs`] 里的一个键。
///
/// 序列化成普通字符串（`"autoselect"` 或配置项名字），因此配置文件里直接写即可。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum JavaChoice {
    /// 写成 `"autoselect"`。
    #[default]
    Auto,
    /// [`JavaConfigs`] 里某一项的名字。
    Named(String),
}

impl JavaChoice {
    pub fn as_str(&self) -> &str {
        match self {
            JavaChoice::Auto => JAVA_AUTOSELECT,
            JavaChoice::Named(name) => name,
        }
    }

    /// 是否自动选择。
    pub fn is_auto(&self) -> bool {
        matches!(self, JavaChoice::Auto)
    }
}

impl From<&str> for JavaChoice {
    fn from(raw: &str) -> Self {
        if raw == JAVA_AUTOSELECT {
            JavaChoice::Auto
        } else {
            JavaChoice::Named(raw.to_string())
        }
    }
}

impl fmt::Display for JavaChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for JavaChoice {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for JavaChoice {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(JavaChoice::from(
            String::deserialize(deserializer)?.as_str(),
        ))
    }
}

/// 一个探测过的 java 可执行文件。
#[derive(Debug, Clone)]
pub struct JavaOption {
    /// java 可执行文件。
    pub path: PathBuf,
    /// 厂商 / 类型，`java -version` 输出的第一个词（例如 `openjdk`）。
    pub type_: String,
    /// JVM 所在的操作系统（`os.name`）。
    pub os: Os,
    /// JVM 架构（`os.arch`）。
    pub arch: Arch,
    /// 大版本号（`1.8` ⇒ 8，`17.0.2` ⇒ 17）。
    pub major_version: u32,
    /// 完整版本字符串（`17.0.2`）。
    pub version_str: String,
}

impl JavaOption {
    /// 探测一个 java 可执行文件。
    ///
    /// 用 `-XshowSettings:properties -version`：一次调用既能拿到版本横幅，又能拿到
    /// `os.name` / `os.arch`（`java -version` 给不出 `linux` / `x86_64` 这种平台信息）。
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
        let major_version = major_version_of(&version_str).unwrap_or(0);
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
        let os = Os::from_java_name(&property("os.name = ")).unwrap_or_else(Os::current);

        Ok(Self {
            path,
            type_,
            os,
            arch,
            major_version,
            version_str,
        })
    }

    /// 探测是否得到了可用的版本号。
    pub fn is_usable(&self) -> bool {
        self.major_version > 0
    }

    /// 探测结果对应的配置项名字。
    ///
    /// 格式为 `{type}-{os}-{arch}-{version}-{hash}`，`hash` 是规范化可执行文件路径的
    /// SHA-1 前 4 字节，用来区分同版本、同平台但安装在不同路径的 Java。
    pub fn config_name(&self) -> String {
        format!(
            "{}-{}-{}-{}-{}",
            self.type_,
            self.os.mc_name(),
            self.arch.name(),
            self.version_str,
            path_hash(&self.path)
        )
    }
}

/// 规范化路径的 SHA-1 前 4 字节（8 位十六进制）。
fn path_hash(path: &Path) -> String {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let digest = Sha1::digest(canonical.as_os_str().as_encoded_bytes());
    format!(
        "{:02x}{:02x}{:02x}{:02x}",
        digest[0], digest[1], digest[2], digest[3]
    )
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

/// 扫描目录里的 `java` 可执行文件。
///
/// **只返回结果，不写配置**；扫不到或全部探测失败时返回空表。
pub async fn find_java(roots: &[impl AsRef<Path>]) -> JavaConfigs {
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

    let mut configs = JavaConfigs::new();
    for path in candidates.values() {
        // 跳过探测失败或版本不可用的文件：一个野文件不该让整次搜索失败
        let Ok(java) = JavaOption::probe(path.clone()).await else {
            continue;
        };
        if !java.is_usable() {
            continue;
        }
        let base = java.config_name();
        let mut name = base.clone();
        let mut suffix = 2;
        // 短哈希理论上仍可能碰撞；真碰上就再补一个序号，保证配置键唯一。
        while configs.contains_key(&name) {
            name = format!("{base}-{suffix}");
            suffix += 1;
        }
        configs.insert(name, java.path);
    }
    configs
}

/// 按调用者给的选择挑一个 java。
///
/// * [`JavaChoice::Named`]：直接用配置表里的那一项；
/// * [`JavaChoice::Auto`]：找与 `required_major` 最接近的（优先完全相等）。
///
/// 配置表为空且是自动选择时，会现场扫一遍 [`default_java_roots`]（**不写回配置**）。
pub async fn select_java(
    required_major: u32,
    choice: &JavaChoice,
    configs: &JavaConfigs,
) -> Result<JavaOption> {
    let scanned = if choice.is_auto() && configs.is_empty() {
        Some(find_java(&default_java_roots()).await)
    } else {
        None
    };
    let configs = scanned.as_ref().unwrap_or(configs);
    if configs.is_empty() {
        return Err(anyhow!(
            "没有找到可用的 Java，请先安装 Java 或提供 Java 路径"
        ));
    }

    let name = match choice {
        JavaChoice::Named(name) => name.clone(),
        JavaChoice::Auto => auto_select_java(required_major, configs).await?,
    };

    let path = configs
        .get(&name)
        .cloned()
        .ok_or_else(|| anyhow!("没有这个 java 配置: {name}"))?;
    let java = JavaOption::probe(path).await?;
    Ok(java)
}

/// `autoselect` 的搜索规则：完全相等优先，否则"至少满足要求里最接近的"，
/// 再否则取第一个可用的。
///
/// 探测失败或版本不可用的配置项会被跳过，不会因为一条坏配置中断选择。
async fn auto_select_java(required_major: u32, configs: &JavaConfigs) -> Result<String> {
    let mut best: Option<String> = None;
    let mut best_diff = u32::MAX;
    let mut first_usable: Option<String> = None;
    for (name, path) in configs {
        let Ok(java) = JavaOption::probe(path.clone()).await else {
            continue;
        };
        if !java.is_usable() {
            continue;
        }
        first_usable.get_or_insert_with(|| name.clone());
        if java.major_version == required_major {
            return Ok(name.clone());
        }
        // `major_version` 是无符号数，这里不需要 `abs()`
        let diff = java.major_version.saturating_sub(required_major);
        if java.major_version >= required_major && diff < best_diff {
            best_diff = diff;
            best = Some(name.clone());
        }
    }
    best.or(first_usable)
        .ok_or_else(|| anyhow!("没有找到可用的 Java，请先安装 Java 或提供 Java 路径"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_choice_round_trips_as_a_plain_string() {
        assert_eq!(
            serde_json::to_string(&JavaChoice::Auto).unwrap(),
            "\"autoselect\""
        );
        assert_eq!(
            serde_json::to_string(&JavaChoice::Named("jdk17".to_string())).unwrap(),
            "\"jdk17\""
        );
        assert_eq!(JavaChoice::from("autoselect"), JavaChoice::Auto);
        assert_eq!(
            JavaChoice::from("jdk17"),
            JavaChoice::Named("jdk17".to_string())
        );
        assert_eq!(JavaChoice::Auto.as_str(), JAVA_AUTOSELECT);
        assert!(JavaChoice::Auto.is_auto());
        assert_eq!(JavaChoice::Named("x".into()).to_string(), "x");
    }

    #[test]
    fn config_name_uses_os_arch_and_a_stable_path_hash() {
        let java = JavaOption {
            path: PathBuf::from("/opt/jdk/bin/java"),
            type_: "openjdk".to_string(),
            os: Os::Linux,
            arch: Arch::Aarch64,
            major_version: 17,
            version_str: "17.0.2".to_string(),
        };
        let name = java.config_name();
        assert!(name.starts_with("openjdk-linux-aarch64-17.0.2-"), "{name}");
        let hash = name.rsplit_once('-').unwrap().1;
        assert_eq!(hash.len(), 8);
        assert!(hash.chars().all(|ch| ch.is_ascii_hexdigit()));
        assert_eq!(name, java.config_name());
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
    async fn find_java_returns_an_empty_table_when_nothing_is_found() {
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
            found.values().next().unwrap(),
            &root.join("java"),
            "符号链接本身应作为配置路径"
        );
        assert!(
            found
                .keys()
                .next()
                .unwrap()
                .starts_with("openjdk-linux-x86_64-17.0.8-"),
            "{found:?}"
        );

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

    #[tokio::test]
    async fn selecting_without_any_java_is_an_error() {
        let configs = JavaConfigs::new();
        let err = select_java(17, &JavaChoice::Named("nope".into()), &configs)
            .await
            .unwrap_err();
        // 有配置表时不会去扫描，直接报"没有这一项"
        assert!(format!("{err:#}").contains("没有找到可用的 Java"));
    }

    #[tokio::test]
    async fn auto_selection_skips_broken_entries() {
        let mut configs = JavaConfigs::new();
        configs.insert("broken".to_string(), PathBuf::from("/nonexistent/java"));
        let err = select_java(17, &JavaChoice::Auto, &configs)
            .await
            .unwrap_err();
        assert!(format!("{err:#}").contains("没有找到可用的 Java"));
    }

    #[tokio::test]
    async fn a_named_choice_must_exist() {
        let mut configs = JavaConfigs::new();
        configs.insert("jdk".to_string(), PathBuf::from("/nonexistent/java"));
        let err = select_java(17, &JavaChoice::Named("other".into()), &configs)
            .await
            .unwrap_err();
        assert!(format!("{err:#}").contains("没有这个 java 配置"));
        // 存在的名字会真的去执行 → 报"无法执行"
        let err = select_java(17, &JavaChoice::Named("jdk".into()), &configs)
            .await
            .unwrap_err();
        assert!(format!("{err:#}").contains("无法执行"));
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn the_system_java_can_be_probed() {
        let Ok(java) = JavaOption::probe("/usr/bin/java").await else {
            return; // 这台机器没有 java 就跳过
        };
        if java.is_usable() {
            assert!(matches!(
                java.arch,
                Arch::X86 | Arch::X86_64 | Arch::Aarch64
            ));
            assert!(matches!(java.os, Os::Windows | Os::Linux | Os::MacOs));
            assert!(!java.config_name().is_empty());
        }
    }
}
