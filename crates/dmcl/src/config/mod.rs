//! 配置。
//!
//! 解析与合并全交给 `serde`：嵌套结构体直接对应嵌套 JSON，`#[serde(default)]`
//! 表示「缺键保留默认值」。加载时逐项容错：不合法的配置项回退成默认值并汇总成
//! [`ConfigError`]，缺键仍然静默用默认值。
//!
//! **没有全局 `config` 变量**：[`Config`] 就是一个普通值；CLI 直接拿着它，GUI 把它放进
//! `gtk4::app::config_store` 的共享仓库、给各页面发只读句柄。各业务 crate 都不知道配置
//! 文件的存在（见各自的 `*Options`）。
//!
//! 全局配置分为 `launch` / `accounts` / `javas` / `minecraft_roots` / `minecraft_root` /
//! `download` 六部分；每个实例还可以在
//! `<minecraft_root>/versions/<version_name>/dmcl.json` 里写一个只含 `launch` 的覆盖层。
//!
//! ```json
//! {
//!   "launch": { "java": "autoselect", "account": "player", … },
//!   "accounts": { "player": { "type": "offline", "username": "player" } },
//!   "javas": {
//!     "jdk-17": {
//!       "path": "/usr/lib/jvm/java-17/bin/java",
//!       "major": 17,
//!       "arch": "x86_64"
//!     }
//!   },
//!   "minecraft_roots": {
//!     "当前目录": ".minecraft",
//!     "官方启动器": "…"
//!   },
//!   "minecraft_root": "当前目录",
//!   "download": { "max_tasks": 64, "skip_music": false, … }
//! }
//! ```
//!
//! 动态表（`accounts` / `javas` / `minecraft_roots`）用 [`IndexMap`] 保存，顺序与配置文件一致。
//!
//! 各配置段的默认值、解析与校验分散在 `account.rs` / `java.rs` / `root.rs` /
//! `launch.rs` / `download.rs`；本模块只负责整份文档的读写、逐项容错与跨段编排。

mod account;
mod download;
mod java;
mod launch;
mod root;

pub use account::AccountConfig;
pub use download::DownloadConfig;
pub use java::{
    configs_from_details, merge_javas, same_java_path, unique_java_name, JavaChoice, JavaConfig,
    JavaConfigs,
};
pub use launch::{instance_config_path, LaunchConfig, LoadedInstanceConfig};

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use dmcl_install::InstallOptions;
use dmcl_mcmeta::layout::absolute;
use dmcl_mcmeta::source::DownloadSource;
use indexmap::IndexMap;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// 整份全局配置文档。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub launch: LaunchConfig,
    pub accounts: IndexMap<String, AccountConfig>,
    pub javas: JavaConfigs,
    /// 具名根目录：名字 → 原始路径。顺序与配置文件一致。
    pub minecraft_roots: IndexMap<String, PathBuf>,
    pub minecraft_root: String,
    pub download: DownloadConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            launch: LaunchConfig::default(),
            accounts: account::default_accounts(),
            javas: JavaConfigs::new(),
            minecraft_roots: root::default_roots(),
            minecraft_root: root::CURRENT_ROOT_NAME.to_string(),
            download: DownloadConfig::default(),
        }
    }
}

/// 一次配置加载的结果。
#[derive(Debug, Clone)]
pub struct LoadedConfig {
    /// 已经逐项回退成默认值的配置。
    pub config: Config,
    /// 加载中发现的问题；`None` 表示配置完全正常。
    pub error: Option<ConfigError>,
}

/// 配置加载中发现的问题汇总（可能有多条）。
#[derive(Debug, Clone)]
pub struct ConfigError {
    /// 出问题的配置文件。
    pub path: PathBuf,
    /// 逐条给用户看的警告。
    pub warnings: Vec<ConfigWarning>,
}

/// 一条配置警告：主文字说处理结果，`detail` 小字说错误类型 / 细节。
#[derive(Debug, Clone)]
pub struct ConfigWarning {
    /// 主文字，例如「配置项「launch.window_width」使用默认值 854」。
    pub message: String,
    /// 小字，例如「值不合法：invalid value: integer `-1`, expected u32」。
    pub detail: String,
}

impl std::fmt::Display for ConfigWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}（{}）", self.message, self.detail)
    }
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "配置文件 {} 有 {} 处问题，请修正后重试",
            self.path.display(),
            self.warnings.len()
        )
    }
}

impl std::error::Error for ConfigError {}

/// 「整份配置回退默认值」类的警告（读文件 / JSON 语法 / 顶层结构 / 整体校验）。
fn whole_default_warning(detail: String) -> ConfigWarning {
    ConfigWarning {
        message: "整份配置使用默认值".to_string(),
        detail,
    }
}

impl Config {
    /// 配置文件路径：`<平台配置目录>/dmcl/dmcl.json`。
    ///
    /// 基准目录来自 [`dirs`]：
    ///
    /// * Linux —— `$XDG_CONFIG_HOME` 或 `~/.config`
    /// * Windows —— `%APPDATA%`
    /// * macOS —— `~/Library/Application Support`
    pub fn path() -> PathBuf {
        match dirs::config_dir() {
            Some(base) => absolute(base.join("dmcl").join("dmcl.json")),
            // 极端环境下拿不到平台配置目录时，回落到当前目录下的 dmcl/dmcl.json
            None => absolute(Path::new("dmcl").join("dmcl.json")),
        }
    }

    /// 读配置（文件不存在就用默认值）。
    ///
    /// 配置项本身的问题不会让加载失败：逐项回退默认值，汇总到
    /// [`LoadedConfig::error`] 里交给前端处理。
    pub async fn load(path: Option<&Path>) -> LoadedConfig {
        let path = path.map(Path::to_path_buf).unwrap_or_else(Self::path);
        let (config, warnings) = match tokio::fs::read_to_string(&path).await {
            Ok(text) => Self::parse(&text),
            // 文件不存在就用默认值；其它读错误（权限等）也回退默认值并报警告
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                (Config::default(), Vec::new())
            }
            Err(err) => (
                Config::default(),
                vec![whole_default_warning(format!("无法读取配置文件：{err}"))],
            ),
        };
        Self::into_loaded(config, path, warnings)
    }

    /// [`Config::load`] 的同步版本，只在**启动时**读一次配置。
    ///
    /// GUI 必须在 GLib 主循环启动前拿到配置，CLI 也走同一份；其余地方都用 [`Config::load`]。
    pub fn load_sync(path: Option<&Path>) -> LoadedConfig {
        let path = path.map(Path::to_path_buf).unwrap_or_else(Self::path);
        let (config, warnings) = match std::fs::read_to_string(&path) {
            Ok(text) => Self::parse(&text),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                (Config::default(), Vec::new())
            }
            Err(err) => (
                Config::default(),
                vec![whole_default_warning(format!("无法读取配置文件：{err}"))],
            ),
        };
        Self::into_loaded(config, path, warnings)
    }

    /// 把警告包成 [`LoadedConfig`]（`load` / `load_sync` 共用）。
    fn into_loaded(config: Self, path: PathBuf, warnings: Vec<ConfigWarning>) -> LoadedConfig {
        let error = (!warnings.is_empty()).then_some(ConfigError { path, warnings });
        LoadedConfig { config, error }
    }

    /// 解析配置文本（`load` / `load_sync` 共用）。
    ///
    /// 逐项容错：坏掉的字段 / 段 / 整份文档都回退默认值，并把原因写进返回的警告列表；
    /// 只有缺键才静默用默认值。
    fn parse(text: &str) -> (Self, Vec<ConfigWarning>) {
        let mut warnings = Vec::new();
        let value: serde_json::Value = match serde_json::from_str(text) {
            Ok(value) => value,
            Err(err) => {
                warnings.push(whole_default_warning(format!("无法解析 JSON：{err}")));
                return (Self::default(), warnings);
            }
        };
        let default = Self::default();
        let mut document = match serde_json::to_value(&default) {
            Ok(serde_json::Value::Object(document)) => document,
            // 默认值总能序列化成对象；真出意外就整体退回默认
            _ => {
                warnings.push(ConfigWarning {
                    message: "整份配置使用内置默认值".to_string(),
                    detail: "无法展开默认配置".to_string(),
                });
                return (default, warnings);
            }
        };
        let serde_json::Value::Object(sections) = value else {
            warnings.push(whole_default_warning(
                "配置文件的顶层不是 JSON 对象".to_string(),
            ));
            return (Self::default(), warnings);
        };

        for (section, body) in sections {
            match section.as_str() {
                "launch" => {
                    document["launch"] = parse_section::<LaunchConfig>(
                        "launch",
                        body,
                        &document["launch"],
                        &mut warnings,
                    );
                }
                "download" => {
                    document["download"] = parse_section::<DownloadConfig>(
                        "download",
                        body,
                        &document["download"],
                        &mut warnings,
                    );
                }
                "accounts" => {
                    let table = account::parse_accounts(body, &mut warnings);
                    document["accounts"] = serde_json::to_value(table).unwrap_or_default();
                }
                "javas" => {
                    let table = java::parse_javas(body, &mut warnings);
                    document["javas"] = serde_json::to_value(table).unwrap_or_default();
                }
                "minecraft_roots" => {
                    let roots = root::parse_roots(body, &mut warnings);
                    document["minecraft_roots"] = serde_json::to_value(roots).unwrap_or_default();
                }
                "minecraft_root" => {
                    let fallback = document["minecraft_root"].clone();
                    document["minecraft_root"] =
                        root::parse_root_name(body, fallback, &mut warnings);
                }
                _ => {
                    warnings.push(ConfigWarning {
                        message: format!("配置段「{section}」已忽略"),
                        detail: "未知配置段".to_string(),
                    });
                }
            }
        }

        let mut config = match serde_json::from_value::<Self>(serde_json::Value::Object(document)) {
            Ok(config) => config,
            Err(err) => {
                warnings.push(whole_default_warning(format!("配置整体校验失败：{err}")));
                return (Self::default(), warnings);
            }
        };
        warnings.extend(config.validate_selections());
        (config, warnings)
    }

    /// 校验全局选择项：根目录、账户、Java。失效项回退并产生警告。
    fn validate_selections(&mut self) -> Vec<ConfigWarning> {
        let mut warnings = self.validate_roots();
        warnings.extend(launch::validate_launch_choices(
            &mut self.launch,
            &self.accounts,
            &self.javas,
        ));
        warnings
    }

    /// 持久化整份配置。
    ///
    /// 先写同目录的临时文件，再 rename 覆盖，读者不会看到半截 JSON。
    pub async fn save(&self) -> Result<()> {
        self.save_to(&Self::path()).await
    }

    /// 把原配置文件备份为同目录的 `dmcl.old.json`，再持久化整份配置。
    ///
    /// 原文件不存在时只写入，不产生备份。
    pub async fn save_with_backup(&self) -> Result<()> {
        self.save_with_backup_to(&Self::path()).await
    }

    async fn save_to(&self, path: &Path) -> Result<()> {
        let text = serde_json::to_string_pretty(self)?;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .with_context(|| format!("无法创建配置目录 {}", parent.display()))?;
            restrict_dir(parent)
                .await
                .with_context(|| format!("无法设置配置目录权限 {}", parent.display()))?;
        }

        // 先写同目录的临时文件，再 rename 覆盖：读者只会看到旧文件或完整的新文件。
        let temp = temp_path(path);
        let result = async {
            tokio::fs::write(&temp, text).await?;
            restrict_file(&temp).await?;
            tokio::fs::rename(&temp, path).await
        }
        .await;
        if let Err(err) = result {
            let _ = tokio::fs::remove_file(&temp).await;
            return Err(err).with_context(|| format!("无法写入配置文件 {}", path.display()));
        }
        Ok(())
    }

    async fn save_with_backup_to(&self, path: &Path) -> Result<()> {
        let backup = path.with_file_name("dmcl.old.json");
        match tokio::fs::copy(path, &backup).await {
            Ok(_) => {
                restrict_file(&backup)
                    .await
                    .with_context(|| format!("无法设置配置备份权限 {}", backup.display()))?;
            }
            // 原文件不存在时没有可备份的内容。
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => {
                return Err(err).with_context(|| {
                    format!(
                        "无法备份配置文件 {} 到 {}",
                        path.display(),
                        backup.display()
                    )
                });
            }
        }
        self.save_to(path).await
    }

    /// 配置 + 传入的启动配置 → [`InstallOptions`]（含「用哪个 java 跑安装器 processor」）。
    pub fn install_options(
        &self,
        source: DownloadSource,
        launch: &LaunchConfig,
        root: &Path,
    ) -> Result<InstallOptions> {
        Ok(InstallOptions::new(root.to_path_buf(), source)
            .with_download(self.download_options())
            .with_skip_music(self.download.skip_music)
            .with_java(Arc::new(java::ConfigJava {
                choice: launch.java.clone(),
                configs: self.javas.clone(),
            })))
    }
}

/// 原子写用的临时文件：目标同目录的 `<stem>.<pid>.<ext>`。
///
/// 例：`dmcl.json` → `dmcl.<pid>.json`。不同进程的 PID 不同，不会争用同一个临时文件。
fn temp_path(path: &Path) -> PathBuf {
    let mut name = path.file_stem().map_or_else(
        || std::ffi::OsString::from("dmcl"),
        |stem| stem.to_os_string(),
    );
    name.push(format!(".{}", std::process::id()));
    if let Some(extension) = path.extension() {
        name.push(".");
        name.push(extension);
    }
    path.with_file_name(name)
}

/// 收紧目录权限；Unix 下仅当前用户可读写执行。
async fn restrict_dir(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).await?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// 收紧文件权限；Unix 下仅当前用户可读写。
async fn restrict_file(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).await?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// 按字段逐项覆盖一个配置段；坏字段回退到传入的默认段。
fn parse_section<T>(
    section: &str,
    body: serde_json::Value,
    fallback: &serde_json::Value,
    warnings: &mut Vec<ConfigWarning>,
) -> serde_json::Value
where
    T: DeserializeOwned + Serialize,
{
    let serde_json::Value::Object(entries) = body else {
        warnings.push(ConfigWarning {
            message: format!("配置段「{section}」使用默认值"),
            detail: "该段不是 JSON 对象".to_string(),
        });
        return fallback.clone();
    };
    let Some(defaults) = fallback.as_object() else {
        return fallback.clone();
    };
    let mut current = defaults.clone();

    for (field, raw) in entries {
        if !defaults.contains_key(&field) {
            warnings.push(ConfigWarning {
                message: format!("配置项「{section}.{field}」已忽略"),
                detail: "未知配置项".to_string(),
            });
            continue;
        }
        let fallback_value = current[field.as_str()].clone();
        let mut candidate = current.clone();
        candidate.insert(field.clone(), raw);
        match serde_json::from_value::<T>(serde_json::Value::Object(candidate)) {
            Ok(value) => match serde_json::to_value(value) {
                Ok(serde_json::Value::Object(object)) => current = object,
                _ => {
                    warnings.push(ConfigWarning {
                        message: format!("配置项「{section}.{field}」使用默认值 {fallback_value}"),
                        detail: "值不合法：无法写回配置".to_string(),
                    });
                }
            },
            Err(err) => warnings.push(ConfigWarning {
                message: format!("配置项「{section}.{field}」使用默认值 {fallback_value}"),
                detail: format!("值不合法：{err}"),
            }),
        }
    }

    serde_json::Value::Object(current)
}

/// 解析一张动态表：条目逐个校验，坏条目丢弃；表整体替换默认表。
fn parse_table<T>(
    section: &str,
    body: serde_json::Value,
    fallback: IndexMap<String, T>,
    warnings: &mut Vec<ConfigWarning>,
) -> IndexMap<String, T>
where
    T: DeserializeOwned,
{
    let serde_json::Value::Object(entries) = body else {
        warnings.push(ConfigWarning {
            message: format!("配置段「{section}」使用默认值"),
            detail: "该段不是 JSON 对象".to_string(),
        });
        return fallback;
    };
    let mut table = IndexMap::with_capacity(entries.len());
    for (name, raw) in entries {
        match serde_json::from_value::<T>(raw) {
            Ok(value) => {
                table.insert(name, value);
            }
            Err(err) => warnings.push(ConfigWarning {
                message: format!("配置项「{section}.{name}」已忽略"),
                detail: format!("值不合法：{err}"),
            }),
        }
    }
    table
}

/// 每个测试独占的临时目录。
#[cfg(test)]
pub(crate) fn temp_dir(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!("dmcl-config-{label}-{}", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::root::CURRENT_ROOT_NAME;
    use super::*;
    use dmcl_mcmeta::source::MirrorPolicy;

    #[test]
    fn the_global_config_lives_in_a_dmcl_directory() {
        assert!(Config::path().ends_with(Path::new("dmcl").join("dmcl.json")));
    }

    #[tokio::test]
    async fn save_is_atomic_and_leaves_no_temporary_file() {
        let dir = temp_dir("atomic");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("dmcl.json");

        Config::default().save_to(&path).await.unwrap();

        let temp = dir.join(format!("dmcl.{}.json", std::process::id()));
        assert_eq!(temp_path(&path), temp);
        assert!(!temp.exists());
        let saved: Config = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            saved.launch.window_width,
            Config::default().launch.window_width
        );

        // 让临时文件路径先被目录占住，写入必然失败；旧文件必须保持完整。
        std::fs::write(&path, r#"{"launch":{"window_width":1024}}"#).unwrap();
        std::fs::create_dir_all(&temp).unwrap();
        assert!(Config::default().save_to(&path).await.is_err());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            r#"{"launch":{"window_width":1024}}"#
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reads_the_global_layout() {
        let text = r#"{
            "launch": {
                "java": "jdk",
                "account": "steve",
                "window_width": 1280
            },
            "accounts": {
                "steve": { "type": "offline", "username": "Steve" }
            },
            "javas": {
                "jdk": {
                    "path": "/usr/lib/jvm/java-17/bin/java",
                    "major": 17,
                    "arch": "x86_64"
                }
            },
            "minecraft_roots": { "test": "/mc" },
            "minecraft_root": "test",
            "download": { "mirror": "mirror-first", "skip_music": true }
        }"#;
        let (config, warnings) = Config::parse(text);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(config.launch.java, JavaChoice::Named("jdk".to_string()));
        assert_eq!(config.launch.account.as_deref(), Some("steve"));
        assert_eq!(config.launch.window_width, 1280);
        assert_eq!(
            config.accounts.get("steve"),
            Some(&AccountConfig::Offline {
                username: "Steve".to_string()
            })
        );
        assert_eq!(
            config.javas.get("jdk"),
            Some(&JavaConfig {
                path: PathBuf::from("/usr/lib/jvm/java-17/bin/java"),
                major: 17,
                arch: dmcl_mcmeta::os::Arch::X86_64,
            })
        );
        assert_eq!(config.root_path("test").unwrap(), absolute("/mc"));
        assert_eq!(config.minecraft_root, "test");
        assert_eq!(config.download.mirror, MirrorPolicy::MirrorFirst);
        assert!(config.download.skip_music);
        // 缺键保留默认值
        assert_eq!(config.launch.window_height, 480);
        assert_eq!(config.launch.min_memory, None);
        assert_eq!(config.launch.max_memory, None);
        assert_eq!(config.download.max_tasks, 64);
    }

    #[test]
    fn memory_fields_accept_null_and_explicit_numbers() {
        let (config, warnings) =
            Config::parse(r#"{"launch": {"min_memory": null, "max_memory": 4096}}"#);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(config.launch.min_memory, None);
        assert_eq!(config.launch.max_memory, Some(4096));
    }

    #[test]
    fn invalid_fields_fall_back_to_their_defaults_with_warnings() {
        let (config, warnings) = Config::parse(
            r#"{
                "launch": { "window_width": -1, "window_height": 720, "max_memory": -2048 },
                "download": { "max_tasks": 8 }
            }"#,
        );
        assert_eq!(config.launch.window_width, 854);
        assert_eq!(config.launch.max_memory, None);
        assert_eq!(config.launch.window_height, 720);
        assert_eq!(config.download.max_tasks, 8);
        assert_eq!(warnings.len(), 2);
        assert!(warnings
            .iter()
            .any(|warning| warning.message.contains("launch.window_width")));
        assert!(warnings
            .iter()
            .any(|warning| warning.message.contains("launch.max_memory")));
        assert!(warnings
            .iter()
            .all(|warning| warning.detail.contains("值不合法")));
    }

    #[test]
    fn old_sections_and_fields_are_ignored_with_warnings() {
        let (config, warnings) = Config::parse(
            r#"{
                "account": { "username": "steve" },
                "launch": {
                    "java_configs": { "jdk": "/usr/bin/java" },
                    "minecraft_root_path": "/mc",
                    "window_width": 1024
                },
                "download": { "i_dont_need_music": true }
            }"#,
        );
        assert_eq!(config.launch.window_width, 1024);
        assert_eq!(config.launch.account.as_deref(), Some("player"));
        assert!(config.javas.is_empty());
        assert_eq!(config.minecraft_root, CURRENT_ROOT_NAME);
        assert!(!config.download.skip_music);
        assert_eq!(warnings.len(), 4);
        assert!(warnings.iter().any(|warning| {
            warning.message.contains("account") && warning.detail.contains("未知配置段")
        }));
        assert!(warnings
            .iter()
            .any(|warning| warning.message.contains("launch.java_configs")));
        assert!(warnings
            .iter()
            .any(|warning| warning.message.contains("launch.minecraft_root_path")));
        assert!(warnings
            .iter()
            .any(|warning| warning.message.contains("download.i_dont_need_music")));
    }

    #[test]
    fn configuration_round_trips_through_a_file() {
        let dir = temp_dir("roundtrip");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("dmcl.json");

        let mut config = Config::default();
        config.accounts.insert(
            "steve".to_string(),
            AccountConfig::Offline {
                username: "Steve".to_string(),
            },
        );
        config.launch.account = Some("steve".to_string());
        config.download.mirror = MirrorPolicy::MirrorFirst;
        config.download.skip_music = true;
        config
            .minecraft_roots
            .insert("test".to_string(), PathBuf::from("mc-root"));
        config.minecraft_root = "test".to_string();

        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, serde_json::to_string_pretty(&config).unwrap()).unwrap();

        let runtime = tokio::runtime::Runtime::new().unwrap();
        let loaded = runtime.block_on(Config::load(Some(&path)));
        assert!(loaded.error.is_none(), "{:?}", loaded.error);
        let loaded = loaded.config;
        assert_eq!(
            loaded.accounts.get("steve"),
            Some(&AccountConfig::Offline {
                username: "Steve".to_string()
            })
        );
        assert_eq!(loaded.download.mirror, MirrorPolicy::MirrorFirst);
        assert!(loaded.download.skip_music);
        assert_eq!(loaded.root_path("test").unwrap(), absolute("mc-root"));
        assert_eq!(
            loaded.minecraft_roots.get("test"),
            Some(&PathBuf::from("mc-root"))
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn save_with_backup_keeps_the_previous_file() {
        let dir = temp_dir("backup");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("dmcl.json");
        let backup = dir.join("dmcl.old.json");
        std::fs::write(&path, r#"{"launch":{"window_width":1024}}"#).unwrap();

        Config::default().save_with_backup_to(&path).await.unwrap();

        assert_eq!(
            std::fs::read_to_string(&backup).unwrap(),
            r#"{"launch":{"window_width":1024}}"#
        );
        let saved: Config = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            saved.launch.window_width,
            Config::default().launch.window_width
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn save_with_backup_does_not_create_a_backup_for_a_missing_file() {
        let dir = temp_dir("backup-missing");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("dmcl.json");

        Config::default().save_with_backup_to(&path).await.unwrap();

        assert!(path.exists());
        assert!(!dir.join("dmcl.old.json").exists());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn load_sync_matches_load() {
        let dir = temp_dir("sync");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("dmcl.json");

        let mut config = Config::default();
        config
            .minecraft_roots
            .insert("test".to_string(), PathBuf::from("mc-root"));
        config.minecraft_root = "test".to_string();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, serde_json::to_string_pretty(&config).unwrap()).unwrap();

        let loaded = Config::load_sync(Some(&path));
        assert!(loaded.error.is_none(), "{:?}", loaded.error);
        assert_eq!(
            loaded.config.root_path("test").unwrap(),
            absolute("mc-root")
        );

        let missing = Config::load_sync(Some(&dir.join("missing.json")));
        assert!(missing.error.is_none());
        assert_eq!(missing.config.launch.account.as_deref(), Some("player"));
        assert_eq!(
            missing.config.default_root_path().unwrap(),
            absolute(".minecraft")
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn load_reports_problems_through_the_error_field() {
        let dir = temp_dir("bad");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("dmcl.json");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, r#"{"launch":{"window_width":-1}}"#).unwrap();

        let loaded = Config::load_sync(Some(&path));
        assert_eq!(loaded.config.launch.window_width, 854);
        let error = loaded.error.as_ref().expect("应当有警告");
        assert_eq!(error.path, path);
        assert_eq!(error.warnings.len(), 1);
        assert!(error.warnings[0].message.contains("launch.window_width"));
        assert!(error.warnings[0].detail.contains("值不合法"));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn install_options_follow_the_config() {
        let dir = temp_dir("install-options");
        let _ = std::fs::remove_dir_all(&dir);
        let mut config = Config::default();
        config.download.max_tasks = 8;
        config.download.skip_music = true;
        config.javas.insert(
            "jdk".to_string(),
            JavaConfig {
                path: PathBuf::from("/nonexistent/bin/java"),
                major: 17,
                arch: dmcl_mcmeta::os::Arch::X86_64,
            },
        );
        config.launch.java = JavaChoice::Named("jdk".to_string());

        let root = dir.join("mc");
        let install = config
            .install_options(config.download_source(), &config.launch, &root)
            .unwrap();
        assert_eq!(install.root_path, root);
        assert!(install.skip_music);
        assert_eq!(install.download.max_connections, 8);

        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(all(test, unix))]
mod unix_permissions {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn mode(path: &Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[tokio::test]
    async fn save_restricts_the_directory_and_files_to_the_current_user() {
        let dir = temp_dir("permissions");
        let _ = std::fs::remove_dir_all(&dir);
        let config_dir = dir.join("dmcl");
        let path = config_dir.join("dmcl.json");
        let config = Config::default();

        config.save_to(&path).await.unwrap();
        assert_eq!(mode(&config_dir), 0o700);
        assert_eq!(mode(&path), 0o600);

        config.save_with_backup_to(&path).await.unwrap();
        let backup = config_dir.join("dmcl.old.json");
        assert_eq!(mode(&backup), 0o600);
        assert_eq!(mode(&path), 0o600);

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
