//! `javas.*` 段、`launch.java` 选择，以及与 `dmcl-launch` 的转换。
//!
//! 配置表是 `名字 → JavaConfig` 的 [`IndexMap`]；`dmcl-launch` 只认识
//! [`JavaOption`] / [`JavaDetails`]，两者在调用边界上转换。

use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use dmcl_java::{
    auto_select_java, default_java_roots, find_java, JavaDetails, JavaOption, JavaResolver,
};
use dmcl_mcmeta::layout::absolute;
use dmcl_mcmeta::os::{Arch, Os};
use dmcl_task::BoxFuture;
use indexmap::IndexMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use sha1::{Digest, Sha1};

use super::{Config, ConfigWarning, LaunchConfig};

/// 表示「自动选择」的哨兵字符串。
pub const JAVA_AUTOSELECT: &str = "autoselect";

/// `javas` 表里的一项：路径与探测结果缓存。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JavaConfig {
    /// java 可执行文件。
    pub path: PathBuf,
    /// 大版本号（`1.8` ⇒ 8，`17.0.2` ⇒ 17）。
    pub major: u32,
    /// JVM 架构（`os.arch`）。
    pub arch: Arch,
}

impl JavaConfig {
    /// 转成 `dmcl-launch` 只认识的运行时信息。
    pub(crate) fn option(&self) -> JavaOption {
        JavaOption {
            path: self.path.clone(),
            major: self.major,
            arch: self.arch,
        }
    }
}

/// java 配置表：名字 → 配置项，保持顺序。
pub type JavaConfigs = IndexMap<String, JavaConfig>;

/// `launch.java` / `download.java` —— 要么自动选择，要么是 [`JavaConfigs`] 里的一个键。
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

/// 解析 `javas` 表：条目逐个校验，坏条目丢弃；表整体替换默认表。
pub(super) fn parse_javas(body: Value, warnings: &mut Vec<ConfigWarning>) -> JavaConfigs {
    super::parse_table("javas", body, JavaConfigs::new(), warnings)
}

/// `launch.java` / `download.java` 指向不存在的 Java 时回退为自动选择。
fn validate_choice(
    field: &str,
    choice: &mut JavaChoice,
    javas: &JavaConfigs,
) -> Option<ConfigWarning> {
    let JavaChoice::Named(name) = choice else {
        return None;
    };
    if javas.contains_key(name) {
        return None;
    }
    let name = name.clone();
    *choice = JavaChoice::Auto;
    Some(ConfigWarning {
        message: format!("配置项「{field}」指向的 Java 不存在：{name}，已改为 autoselect"),
        detail: "请编辑配置选择已有 Java".to_string(),
    })
}

/// `launch.java` 指向不存在的 Java 时回退为自动选择。
pub(super) fn validate_selection(
    launch: &mut LaunchConfig,
    javas: &JavaConfigs,
) -> Vec<ConfigWarning> {
    validate_choice("launch.java", &mut launch.java, javas)
        .into_iter()
        .collect()
}

/// `download.java` 指向不存在的 Java 时回退为自动选择。
pub(super) fn validate_download_selection(
    download: &mut super::DownloadConfig,
    javas: &JavaConfigs,
) -> Vec<ConfigWarning> {
    validate_choice("download.java", &mut download.java, javas)
        .into_iter()
        .collect()
}

/// 手动指定 Java 时，路径必须仍然存在；选择阶段只检查文件，不执行 Java。
pub(crate) fn ensure_java_path(name: &str, java: &JavaConfig) -> Result<()> {
    if java.path.is_file() {
        Ok(())
    } else {
        Err(anyhow!(
            "Java 配置项「{name}」的可执行文件不存在：{}，请重新搜索 Java",
            java.path.display()
        ))
    }
}

/// 把扫描结果转成配置表：名字按惯例生成，短哈希碰撞时补序号。
pub fn configs_from_details(details: Vec<JavaDetails>) -> JavaConfigs {
    let mut configs = JavaConfigs::new();
    for details in details {
        let name = unique_java_name(&configs, &details);
        let JavaOption { path, major, arch } = details.option;
        configs.insert(name, JavaConfig { path, major, arch });
    }
    configs
}

/// 按探测结果生成一个当前表里唯一的键。
pub fn unique_java_name(javas: &JavaConfigs, details: &JavaDetails) -> String {
    let base = config_name(details);
    let mut name = base.clone();
    let mut suffix = 2;
    // 短哈希理论上仍可能碰撞；真碰上就再补一个序号，保证配置键唯一。
    while javas.contains_key(&name) {
        name = format!("{base}-{suffix}");
        suffix += 1;
    }
    name
}

/// 把扫描结果合并进 `javas`，返回按路径新增了几个。
///
/// 已有名字（手工条目、旧配置里的非哈希名字）全部保留；同一路径不会以不同名字
/// 重复加入。同名条目更新整项，同一路径的其它名字只刷新缓存的版本 / 架构。
pub fn merge_javas(javas: &mut JavaConfigs, found: &JavaConfigs) -> usize {
    let mut added = 0;
    for (name, java) in found {
        let existing_name = javas
            .iter()
            .find(|(_, existing)| same_java_path(&existing.path, &java.path))
            .map(|(existing_name, _)| existing_name.clone());
        if let Some(existing_name) = existing_name {
            if existing_name == *name {
                javas.insert(name.clone(), java.clone());
            } else if let Some(existing) = javas.get_mut(&existing_name) {
                existing.major = java.major;
                existing.arch = java.arch;
            }
            continue;
        }
        if javas.insert(name.clone(), java.clone()).is_none() {
            added += 1;
        }
    }
    added
}

pub fn same_java_path(left: &Path, right: &Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => absolute(left) == absolute(right),
    }
}

impl Config {
    /// 新增 Java；名字非空、不重名，路径也不能为空。
    pub fn add_java(&mut self, name: &str, java: JavaConfig) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            return Err(anyhow!("Java 名称不能为空"));
        }
        if self.javas.contains_key(name) {
            return Err(anyhow!("已存在同名 Java：{name}"));
        }
        if java.path.as_os_str().is_empty() {
            return Err(anyhow!("Java 路径不能为空"));
        }
        self.javas.insert(name.to_string(), java);
        Ok(())
    }

    /// 替换某个 Java 的路径与缓存的版本 / 架构；名字必须已经存在。
    pub fn set_java(&mut self, name: &str, java: JavaConfig) -> Result<()> {
        let Some(slot) = self.javas.get_mut(name) else {
            return Err(anyhow!("Java 不存在：{name}"));
        };
        if java.path.as_os_str().is_empty() {
            return Err(anyhow!("Java 路径不能为空"));
        }
        *slot = java;
        Ok(())
    }

    /// 把 Java 移动到 `to`（移动后的下标）；默认 Java 不变。
    pub fn move_java(&mut self, name: &str, to: usize) -> Result<()> {
        let from = self
            .javas
            .get_index_of(name)
            .ok_or_else(|| anyhow!("Java 不存在：{name}"))?;
        if to >= self.javas.len() {
            return Err(anyhow!("目标位置超出范围：{to}"));
        }
        if from == to {
            return Ok(());
        }

        let java = self
            .javas
            .shift_remove(name)
            .expect("上面已经确认 Java 存在");
        self.javas.shift_insert(to, name.to_string(), java);
        Ok(())
    }

    /// 删除 Java；删掉的正好是启动 / 安装的选择项时改回自动选择。
    pub fn remove_java(&mut self, name: &str) -> Result<()> {
        if !self.javas.contains_key(name) {
            return Err(anyhow!("Java 不存在：{name}"));
        }
        self.javas.shift_remove(name);
        let named = JavaChoice::Named(name.to_string());
        if self.launch.java == named {
            self.launch.java = JavaChoice::Auto;
        }
        if self.download.java == named {
            self.download.java = JavaChoice::Auto;
        }
        Ok(())
    }
}

/// 配置项名字：`{type}-{os}-{arch}-{version}-{hash}`。
///
/// `hash` 是规范化可执行文件路径的 SHA-1 前 4 字节，用来区分同版本、同平台但安装
/// 在不同路径的 Java。
fn config_name(java: &JavaDetails) -> String {
    format!(
        "{}-{}-{}-{}-{}",
        java.type_,
        Os::current().mc_name(),
        java.option.arch.name(),
        java.version_str,
        path_hash(&java.option.path)
    )
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

/// 按配置里的显式名字取 Java；路径必须仍然存在。
fn named_java(name: &str, configs: &JavaConfigs) -> Result<JavaOption> {
    let java = configs
        .get(name)
        .ok_or_else(|| anyhow!("Java 配置项不存在：{name}"))?;
    ensure_java_path(name, java)?;
    Ok(java.option())
}

/// 从配置表里自动选择；只读缓存的版本 / 架构，路径必须仍然存在。
pub(crate) fn configured_java(required_major: u32, configs: &JavaConfigs) -> Option<JavaOption> {
    let options: Vec<JavaOption> = configs
        .values()
        .filter(|java| java.path.is_file())
        .map(JavaConfig::option)
        .collect();
    auto_select_java(required_major, &options)
}

/// 扫描本机，转成可以直接并入配置的条目。
pub(crate) async fn scan_configs() -> JavaConfigs {
    configs_from_details(find_java(&default_java_roots()).await)
}

/// CLI / 安装流程共用的选择逻辑。
///
/// `scan = false` 只认配置；`true` 时配置里没有可用条目才扫描本机（不写回配置）。
pub(crate) async fn resolve_from_config(
    required_major: u32,
    choice: &JavaChoice,
    configs: &JavaConfigs,
    scan: bool,
) -> Result<JavaOption> {
    match choice {
        JavaChoice::Named(name) => named_java(name, configs),
        JavaChoice::Auto => {
            if let Some(java) = configured_java(required_major, configs) {
                return Ok(java);
            }
            if !scan {
                return Err(anyhow!(
                    "配置中没有可用的 Java，请先执行 dmcl search-java 扫描本机 Java"
                ));
            }
            let found = scan_configs().await;
            configured_java(required_major, &found)
                .ok_or_else(|| anyhow!("未找到可用的 Java，请先安装 Java 或在配置中添加 Java 路径"))
        }
    }
}

/// 安装流程用的 Java 解析器：按 `download.java` 选择，自动模式下先读配置、再扫描本机。
///
/// 这里只负责找**现有** Java；找不到时由安装流程用
/// [`dmcl_install::InstallOptions::java_for_or_install`] 下载 JRE，下载进度因此能显示在
/// 安装任务里。与启动时的 `launch.java` 无关，扫描结果也不写回配置。
pub(crate) struct InstallJavaResolver {
    choice: JavaChoice,
    configs: JavaConfigs,
}

impl InstallJavaResolver {
    pub(crate) fn new(choice: JavaChoice, configs: JavaConfigs) -> Self {
        Self { choice, configs }
    }
}

impl JavaResolver for InstallJavaResolver {
    fn resolve<'a>(&'a self, required_major: u32) -> BoxFuture<'a, Result<JavaOption>> {
        Box::pin(async move {
            let java =
                resolve_from_config(required_major, &self.choice, &self.configs, true).await?;
            // 显式指定不按版本要求改选；自动选择必须精确匹配要求的 Java 大版本，
            // 否则交给安装流程下载对应的 JRE。
            if self.choice.is_auto() && java.major != required_major {
                return Err(anyhow!(
                    "没有找到匹配 Java {required_major} 的可用 Java，请先安装或在配置中添加 Java 路径"
                ));
            }
            Ok(java)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn entry(path: &str, major: u32) -> JavaConfig {
        JavaConfig {
            path: PathBuf::from(path),
            major,
            arch: Arch::X86_64,
        }
    }

    fn temp_java_dir(label: &str) -> PathBuf {
        let dir = crate::config::temp_dir(label);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn file_entry(dir: &Path, name: &str, major: u32) -> JavaConfig {
        let path = dir.join(name);
        std::fs::write(&path, b"").unwrap();
        JavaConfig {
            path,
            major,
            arch: Arch::X86_64,
        }
    }

    fn details(path: &str) -> JavaDetails {
        JavaDetails {
            option: JavaOption {
                path: PathBuf::from(path),
                major: 17,
                arch: Arch::X86_64,
            },
            type_: "openjdk".to_string(),
            version_str: "17.0.2".to_string(),
            vendor: "Test".to_string(),
            is_jdk: true,
        }
    }

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
        assert!(JavaChoice::Auto.is_auto());
        assert_eq!(JavaChoice::Named("x".into()).to_string(), "x");
    }

    #[test]
    fn javas_keep_the_file_order() {
        let text = r#"{
            "javas": {
                "b": { "path": "/b/java", "major": 21, "arch": "aarch64" },
                "a": { "path": "/a/java", "major": 17, "arch": "x86_64" }
            }
        }"#;
        let (config, warnings) = Config::parse(text);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(
            config.javas.keys().map(String::as_str).collect::<Vec<_>>(),
            vec!["b", "a"]
        );

        let saved = serde_json::to_string_pretty(&config).unwrap();
        let (reloaded, warnings) = Config::parse(&saved);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(
            reloaded
                .javas
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec!["b", "a"]
        );
        assert_eq!(
            reloaded.javas.get("b"),
            Some(&JavaConfig {
                path: PathBuf::from("/b/java"),
                major: 21,
                arch: Arch::Aarch64,
            })
        );
    }

    #[test]
    fn bad_java_entries_are_dropped() {
        let (config, warnings) = Config::parse(
            r#"{
                "javas": {
                    "good": { "path": "/usr/bin/java", "major": 17, "arch": "x86_64" },
                    "old": "/usr/bin/java",
                    "bad": 1
                }
            }"#,
        );
        assert!(config.javas.contains_key("good"));
        assert!(!config.javas.contains_key("old"));
        assert!(!config.javas.contains_key("bad"));
        for key in ["javas.old", "javas.bad"] {
            assert!(
                warnings.iter().any(|warning| warning.message.contains(key)),
                "缺少 {key} 的警告: {warnings:?}"
            );
        }
    }

    #[test]
    fn a_missing_named_java_falls_back_to_autoselect() {
        let (config, warnings) = Config::parse(r#"{"launch": {"java": "nope"}}"#);
        assert!(config.launch.java.is_auto());
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].message.contains("launch.java"));

        let (config, warnings) = Config::parse(r#"{"download": {"java": "nope"}}"#);
        assert!(config.download.java.is_auto());
        assert!(warnings
            .iter()
            .any(|warning| warning.message.contains("download.java")));
    }

    #[tokio::test]
    async fn install_java_auto_selects_from_usable_config_entries() {
        let dir = temp_java_dir("install-java");
        let configs = JavaConfigs::from([
            ("eight".to_string(), file_entry(&dir, "eight", 8)),
            ("seventeen".to_string(), file_entry(&dir, "seventeen", 17)),
        ]);
        // resolver 只找现有 Java；自动下载由安装流程的 java_for_or_install 负责。
        let resolver = InstallJavaResolver::new(JavaChoice::Auto, configs);

        assert_eq!(
            JavaResolver::resolve(&resolver, 17).await.unwrap().path,
            dir.join("seventeen")
        );
        assert_eq!(
            JavaResolver::resolve(&resolver, 8).await.unwrap().path,
            dir.join("eight")
        );

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn install_java_auto_selects_only_an_exact_major() {
        let dir = temp_java_dir("install-java-exact");
        let configs = JavaConfigs::from([("eight".to_string(), file_entry(&dir, "eight", 8))]);
        let resolver = InstallJavaResolver::new(JavaChoice::Auto, configs);

        // 自动选择要求精确大版本；不匹配时交给 java_for_or_install 下载 JRE。
        assert!(JavaResolver::resolve(&resolver, 17).await.is_err());

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn install_java_honors_an_explicit_choice() {
        let dir = temp_java_dir("install-java-named");
        let configs = JavaConfigs::from([
            ("eight".to_string(), file_entry(&dir, "eight", 8)),
            ("seventeen".to_string(), file_entry(&dir, "seventeen", 17)),
        ]);
        let resolver = InstallJavaResolver::new(JavaChoice::Named("eight".into()), configs);

        // 显式指定就不按版本要求改选，即使 8 低于要求。
        assert_eq!(
            JavaResolver::resolve(&resolver, 17).await.unwrap().path,
            dir.join("eight")
        );

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn adding_java_rejects_empty_or_duplicate_names() {
        let mut config = Config::default();
        config.add_java("jdk", entry("/jdk", 17)).unwrap();
        assert_eq!(config.javas.get("jdk"), Some(&entry("/jdk", 17)));

        assert!(config.add_java("  ", entry("/other", 17)).is_err());
        assert!(config.add_java("jdk", entry("/other", 17)).is_err());
        assert_eq!(config.javas.len(), 1);
    }

    #[test]
    fn changing_java_path_requires_an_existing_name() {
        let mut config = Config::default();
        config.add_java("jdk", entry("/old", 17)).unwrap();
        config.set_java("jdk", entry("/new", 21)).unwrap();
        assert_eq!(config.javas.get("jdk"), Some(&entry("/new", 21)));

        assert!(config.set_java("nope", entry("/new", 21)).is_err());
    }

    #[test]
    fn moving_java_reorders_without_changing_the_default() {
        let mut config = Config::default();
        config.add_java("a", entry("/a", 17)).unwrap();
        config.add_java("b", entry("/b", 21)).unwrap();
        config.launch.java = JavaChoice::Named("b".to_string());

        config.move_java("b", 0).unwrap();
        assert_eq!(
            config.javas.keys().map(String::as_str).collect::<Vec<_>>(),
            vec!["b", "a"]
        );
        assert_eq!(config.launch.java, JavaChoice::Named("b".to_string()));

        assert!(config.move_java("nope", 0).is_err());
        assert!(config.move_java("a", 2).is_err());
    }

    #[test]
    fn removing_the_default_java_falls_back_to_autoselect() {
        let mut config = Config::default();
        config.add_java("jdk", entry("/jdk", 17)).unwrap();
        config.launch.java = JavaChoice::Named("jdk".to_string());
        config.download.java = JavaChoice::Named("jdk".to_string());
        config.remove_java("jdk").unwrap();

        assert!(config.javas.is_empty());
        assert_eq!(config.launch.java, JavaChoice::Auto);
        assert_eq!(config.download.java, JavaChoice::Auto);
        assert!(config.remove_java("jdk").is_err());
    }

    #[test]
    fn merge_keeps_manual_names_and_deduplicates_by_path() {
        let mut javas = JavaConfigs::from([
            ("manual".to_string(), entry("/manual/java", 17)),
            ("same".to_string(), entry("/old/java", 11)),
        ]);

        let found = JavaConfigs::from([
            ("jdk17-hash".to_string(), entry("/jdk17/bin/java", 21)),
            ("manual-hash".to_string(), entry("/manual/java", 21)),
            ("manual".to_string(), entry("/manual/java", 21)),
            ("same".to_string(), entry("/new/java", 17)),
        ]);

        assert_eq!(merge_javas(&mut javas, &found), 1);
        assert_eq!(javas.len(), 3);
        assert!(!javas.contains_key("manual-hash"));
        assert_eq!(javas.get("manual"), Some(&entry("/manual/java", 21)));
        assert_eq!(javas.get("same"), Some(&entry("/new/java", 17)));
        assert_eq!(javas.get("jdk17-hash"), Some(&entry("/jdk17/bin/java", 21)));
    }

    #[test]
    fn scanned_java_names_include_the_path_hash_and_stay_unique() {
        let configs = configs_from_details(vec![details("/opt/jdk")]);
        let (name, java) = configs.first().unwrap();
        let prefix = format!("openjdk-{}-x86_64-17.0.2-", Os::current().mc_name());
        assert!(name.starts_with(&prefix), "{name}");
        let hash = name.rsplit_once('-').unwrap().1;
        assert_eq!(hash.len(), 8);
        assert!(hash.chars().all(|ch| ch.is_ascii_hexdigit()));
        assert_eq!(java, &entry("/opt/jdk", 17));

        let configs = configs_from_details(vec![details("/opt/jdk"), details("/opt/jdk")]);
        assert_eq!(configs.len(), 2);
        let second = configs.keys().nth(1).unwrap();
        assert!(second.ends_with("-2"), "{second}");
    }
}
