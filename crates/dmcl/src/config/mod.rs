//! 配置。
//!
//! 解析与合并全交给 `serde`：嵌套结构体直接对应嵌套 JSON，`#[serde(default)]`
//! 表示"缺键保留默认值"。加载时逐项容错：不合法的配置项回退成默认值并汇总成
//! [`ConfigError`]，缺键仍然静默用默认值。
//!
//! **没有全局 `config` 变量**：[`Config`] 就是一个普通值，由前端（CLI / GUI）持有并传下去；
//! 各业务 crate 都不知道配置文件的存在（见各自的 `*Options`）。
//!
//! 各段拆在子模块（`launch` / `account` / `download`）；交互编辑用的展开 /
//! 写回在 `edit`，java 管理的纯逻辑在 `java`。
//!
//! ```json
//! {
//!   "launch":   { "java": "autoselect", "minecraft_root_path": "/…/.minecraft", … },
//!   "account":  { "online_mode": false, "username": "player" },
//!   "download": { "max_tasks": 64, "mirror": "official-first", … }
//! }
//! ```

mod account;
mod download;
mod edit;
pub(crate) mod java;
mod launch;

pub use account::AccountConfig;
pub use download::DownloadConfig;
pub use edit::{display_value, is_json_value, parse_edited_value};
pub use launch::LaunchConfig;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use dmcl_download::{DownloadOptions, DEFAULT_USER_AGENT};
use dmcl_install::{InstallOptions, JavaProvider};
use dmcl_launch::{
    select_java, JavaChoice, JavaConfigs, LaunchOption, LAUNCHER_NAME, LAUNCHER_VERSION,
};
use dmcl_mcmeta::layout::absolute;
use dmcl_mcmeta::source::DownloadSource;
use serde::{Deserialize, Serialize};

/// 1 MiB。
const MIB: u64 = 1024 * 1024;

/// 整份配置文档。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub launch: LaunchConfig,
    pub account: AccountConfig,
    pub download: DownloadConfig,
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
    /// 配置文件路径：`<平台配置目录>/dmcl.json`。
    ///
    /// 基准目录来自 [`dirs`]：
    ///
    /// * Linux —— `$XDG_CONFIG_HOME` 或 `~/.config`
    /// * Windows —— `%APPDATA%`
    /// * macOS —— `~/Library/Application Support`
    pub fn path() -> PathBuf {
        match dirs::config_dir() {
            Some(base) => absolute(base.join("dmcl.json")),
            // 极端环境下拿不到平台配置目录时，回落到当前目录下的 dmcl.json
            None => absolute("dmcl.json"),
        }
    }

    /// 读配置（文件不存在就用默认值），并把 `.minecraft` 路径补成绝对路径。
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

    /// 补绝对路径并把警告包成 [`LoadedConfig`]（`load` / `load_sync` 共用）。
    fn into_loaded(mut config: Self, path: PathBuf, warnings: Vec<ConfigWarning>) -> LoadedConfig {
        config.absolutize_root();
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
        let mut root = match serde_json::to_value(&default) {
            Ok(serde_json::Value::Object(root)) => root,
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
            if !root.get(&section).is_some_and(serde_json::Value::is_object) {
                warnings.push(ConfigWarning {
                    message: format!("配置段「{section}」已忽略"),
                    detail: "未知配置段".to_string(),
                });
                continue;
            }
            let serde_json::Value::Object(entries) = body else {
                warnings.push(ConfigWarning {
                    message: format!("配置段「{section}」使用默认值"),
                    detail: "该段不是 JSON 对象".to_string(),
                });
                continue;
            };
            let known: Vec<String> = root[section.as_str()]
                .as_object()
                .map(|object| object.keys().cloned().collect())
                .unwrap_or_default();

            for (field, raw) in entries {
                if !known.contains(&field) {
                    warnings.push(ConfigWarning {
                        message: format!("配置项「{section}.{field}」已忽略"),
                        detail: "未知配置项".to_string(),
                    });
                    continue;
                }
                // 用「默认配置 + 这一项原值」试反序列化；失败就保留默认值
                let mut candidate = root.clone();
                candidate[section.as_str()][field.as_str()] = raw.clone();
                match serde_json::from_value::<Self>(serde_json::Value::Object(candidate)) {
                    Ok(_) => root[section.as_str()][field.as_str()] = raw,
                    Err(err) => {
                        let fallback = &root[section.as_str()][field.as_str()];
                        warnings.push(ConfigWarning {
                            message: format!("配置项「{section}.{field}」使用默认值 {fallback}"),
                            detail: format!("值不合法：{err}"),
                        });
                    }
                }
            }
        }

        match serde_json::from_value::<Self>(serde_json::Value::Object(root)) {
            Ok(config) => (config, warnings),
            Err(err) => {
                warnings.push(whole_default_warning(format!("配置整体校验失败：{err}")));
                (Self::default(), warnings)
            }
        }
    }

    /// 把 `.minecraft` 路径补成绝对路径（`load` / `load_sync` 共用）。
    fn absolutize_root(&mut self) {
        self.launch.minecraft_root_path = absolute(&self.launch.minecraft_root_path);
    }

    /// 持久化整份配置。
    pub async fn save(&self) -> Result<()> {
        let path = Self::path();
        let text = serde_json::to_string_pretty(self)?;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .with_context(|| format!("无法创建配置目录 {}", parent.display()))?;
        }
        tokio::fs::write(&path, text)
            .await
            .with_context(|| format!("无法写入配置文件 {}", path.display()))?;
        Ok(())
    }

    /// [`Config::save`] 的同步版本，给 GTK 回调这类不能等待 future 的地方用。
    pub fn save_sync(&self) -> Result<()> {
        let path = Self::path();
        let text = serde_json::to_string_pretty(self)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("无法创建配置目录 {}", parent.display()))?;
        }
        std::fs::write(&path, text)
            .with_context(|| format!("无法写入配置文件 {}", path.display()))?;
        Ok(())
    }

    /// 按配置造一个下载源（镜像策略 + 重试次数 + User-Agent）。
    pub fn download_source(&self) -> DownloadSource {
        DownloadSource::from_policy(self.download.mirror)
            .with_user_agent(format!("{LAUNCHER_NAME}/{LAUNCHER_VERSION}"))
            .with_max_attempts(self.download.max_attempts)
    }

    /// 配置文件里的下载策略 → [`dmcl_download::DownloadOptions`]。
    pub fn download_options(&self) -> DownloadOptions {
        let mut options = DownloadOptions::with_max_connections(self.download.max_tasks)
            .with_chunking(
                self.download.chunk_threshold_mib.max(1) * MIB,
                self.download.chunk_size_mib.max(1) * MIB,
            )
            .with_max_attempts(self.download.max_attempts)
            .with_user_agent(DEFAULT_USER_AGENT);
        options.max_chunks_per_file = self.download.max_chunks_per_file.max(1);
        options
    }

    /// 配置文件 → [`InstallOptions`]（含"用哪个 java 跑安装器 processor"）。
    pub fn install_options(&self, source: DownloadSource) -> InstallOptions {
        InstallOptions::new(self.launch.minecraft_root_path.clone(), source)
            .with_download(self.download_options())
            .with_skip_music(self.download.i_dont_need_music)
            .with_java(Arc::new(ConfigJava {
                choice: self.launch.java.clone(),
                configs: self.launch.java_configs.clone(),
            }))
    }

    /// 配置文件 → [`LaunchOption`]。
    pub fn launch_option(
        &self,
        account: Arc<dyn dmcl_launch::Account>,
        version_name: &str,
    ) -> LaunchOption {
        let mut option = LaunchOption::new(
            account,
            self.launch.minecraft_root_path.clone(),
            version_name,
        )
        .with_java(self.launch.java.clone(), self.launch.java_configs.clone())
        .with_local_layout(self.download.i_dont_need_music);
        option.window_width = self.launch.window_width;
        option.window_height = self.launch.window_height;
        option.min_memory = self.launch.min_memory;
        option.max_memory = self.launch.max_memory;
        option.custom_info = self.launch.custom_info.clone();
        option.additional_jvm = self.launch.additional_jvm.clone();
        option.additional_game = self.launch.additional_game.clone();
        option
    }

    /// 按配置里的账户信息造一个（离线）账号。
    pub fn account(&self) -> Arc<dyn dmcl_launch::Account> {
        // `account.online_mode` 目前仍未被使用：只有离线账号实现。
        Arc::new(dmcl_launch::OfflineAccount::new(
            self.account.username.clone(),
        ))
    }
}

/// 让 `dmcl-install` 用配置里的 java 选择去跑安装器 processor。
struct ConfigJava {
    choice: JavaChoice,
    configs: JavaConfigs,
}

impl JavaProvider for ConfigJava {
    fn java_for<'a>(&'a self, required_major: u32) -> dmcl_task::BoxFuture<'a, Result<PathBuf>> {
        Box::pin(async move {
            let java = select_java(required_major, &self.choice, &self.configs).await?;
            Ok(java.path)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dmcl_mcmeta::source::MirrorPolicy;

    #[test]
    fn reads_the_same_dotted_json_as_the_d_version() {
        // D 里的 `launch_java` ⇒ `launch.java`，`download_mirror` ⇒ `download.mirror`……
        let text = r#"{
            "launch": { "java": "myjdk", "window_width": 1280,
                        "java_configs": { "jdk": "/usr/bin/java" } },
            "download": { "mirror": "mirror-first" }
        }"#;
        let config: Config = serde_json::from_str(text).unwrap();
        assert_eq!(config.launch.java, JavaChoice::Named("myjdk".to_string()));
        assert_eq!(config.launch.window_width, 1280);
        assert_eq!(
            config.launch.java_configs.get("jdk").unwrap(),
            &PathBuf::from("/usr/bin/java")
        );
        assert_eq!(config.download.mirror, MirrorPolicy::MirrorFirst);
        // 缺键保留默认值
        assert_eq!(config.launch.window_height, 480);
        assert_eq!(config.launch.min_memory, 512);
        assert_eq!(config.download.max_tasks, 64);
        assert_eq!(config.download.chunk_size_mib, 4);
        assert_eq!(config.account.username, "player");
    }

    #[test]
    fn serialises_to_the_same_key_layout() {
        let value = serde_json::to_value(Config::default()).unwrap();
        assert!(value["launch"]["java"].is_string());
        assert!(value["launch"]["minecraft_root_path"].is_string());
        assert!(value["account"]["online_mode"].is_boolean());
        assert!(value["download"]["max_tasks"].is_number());
        assert!(value["download"]["mirror"].is_string());
        assert!(value.get("cli").is_none());
    }

    #[test]
    fn empty_document_yields_all_defaults() {
        let config: Config = serde_json::from_str("{}").unwrap();
        assert!(config.launch.java.is_auto());
        assert_eq!(config.launch.custom_info, LAUNCHER_NAME);
        assert_eq!(config.download.mirror, MirrorPolicy::OfficialFirst);
    }

    #[test]
    fn invalid_values_fall_back_to_defaults_with_warnings() {
        // D 的 `int` 会接受这些值并一路传给 JVM
        let (config, warnings) = Config::parse(
            r#"{
                "launch": { "window_width": -1, "window_height": 720, "max_memory": -2048 },
                "download": { "max_tasks": 8 }
            }"#,
        );
        assert_eq!(config.launch.window_width, 854);
        assert_eq!(config.launch.max_memory, 2048);
        // 合法的项照常生效
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
    fn unknown_keys_and_sections_are_ignored_with_warnings() {
        let (config, warnings) = Config::parse(
            r#"{
                "cli": { "anything": true },
                "launch": { "nope": 1, "window_width": 1024 },
                "account": { "username": "steve" }
            }"#,
        );
        assert_eq!(config.launch.window_width, 1024);
        assert_eq!(config.account.username, "steve");
        assert_eq!(warnings.len(), 2);
        assert!(warnings.iter().any(
            |warning| warning.message.contains("cli") && warning.detail.contains("未知配置段")
        ));
        assert!(warnings
            .iter()
            .any(|warning| warning.message.contains("launch.nope")
                && warning.detail.contains("未知配置项")));
    }

    #[test]
    fn a_broken_section_falls_back_as_a_whole() {
        let (config, warnings) =
            Config::parse(r#"{"launch": 1, "account": {"username": "steve"}}"#);
        assert_eq!(config.launch.window_width, 854);
        assert_eq!(config.account.username, "steve");
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].message.contains("launch"));
        assert!(warnings[0].detail.contains("不是 JSON 对象"));
    }

    #[test]
    fn a_broken_nested_field_falls_back_as_a_whole() {
        let (config, warnings) = Config::parse(
            r#"{
                "launch": {
                    "java": "jdk",
                    "java_configs": { "jdk": 1, "jre": "/usr/bin/java" }
                }
            }"#,
        );
        // 对象里有一个坏条目，整个字段回退默认（空表），其它字段保留
        assert!(config.launch.java_configs.is_empty());
        assert_eq!(config.launch.java, JavaChoice::Named("jdk".to_string()));
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].message.contains("launch.java_configs"));
        assert!(warnings[0].detail.contains("值不合法"));
    }

    #[test]
    fn malformed_json_falls_back_to_all_defaults() {
        let (config, warnings) = Config::parse("{ this is not json");
        assert_eq!(config.launch.custom_info, LAUNCHER_NAME);
        assert_eq!(config.download.mirror, MirrorPolicy::OfficialFirst);
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].message, "整份配置使用默认值");
        assert!(warnings[0].detail.contains("无法解析 JSON"));
    }

    #[test]
    fn the_config_maps_onto_the_crate_options() {
        let mut config = Config::default();
        config.launch.minecraft_root_path = PathBuf::from("/mc");
        config.download.max_tasks = 8;
        config.download.chunk_threshold_mib = 2;
        config.download.chunk_size_mib = 1;
        config.download.max_chunks_per_file = 3;
        config.download.max_attempts = 5;
        config.download.i_dont_need_music = true;
        config.launch.window_width = 1920;
        config.account.username = "steve".to_string();

        let download = config.download_options();
        assert_eq!(download.max_connections, 8);
        assert_eq!(download.chunk_threshold, 2 * MIB);
        assert_eq!(download.chunk_size, MIB);
        assert_eq!(download.max_chunks_per_file, 3);
        assert_eq!(download.max_attempts, 5);

        let install = config.install_options(config.download_source());
        assert_eq!(install.root_path, PathBuf::from("/mc"));
        assert!(install.skip_music);
        assert_eq!(install.download.max_connections, 8);

        let option = config.launch_option(config.account(), "1.20.1");
        assert_eq!(option.root_path, PathBuf::from("/mc"));
        assert_eq!(option.window_width, 1920);
        assert!(option.skip_music);
        assert_eq!(option.account.get_name(), "steve");
    }

    #[tokio::test]
    async fn configuration_round_trips_through_a_file() {
        let dir = std::env::temp_dir().join(format!("dmcl-config-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&dir).await;
        let path = dir.join("dmcl.json");

        let mut config = Config::default();
        config.account.username = "tester".to_string();
        config.download.mirror = MirrorPolicy::MirrorFirst;
        // 故意用相对路径：`load` 会把它补成绝对路径（平台无关）
        config.launch.minecraft_root_path = PathBuf::from("mc-root");

        // 手动写到指定路径（`save` 用的是平台配置目录，测试里不碰它）
        tokio::fs::create_dir_all(&dir).await.unwrap();
        tokio::fs::write(&path, serde_json::to_string_pretty(&config).unwrap())
            .await
            .unwrap();

        let loaded = Config::load(Some(&path)).await;
        assert!(loaded.error.is_none());
        let loaded = loaded.config;
        assert_eq!(loaded.account.username, "tester");
        assert_eq!(loaded.download.mirror, MirrorPolicy::MirrorFirst);
        // 相对路径会被补成绝对路径（相对于进程当前目录）
        assert_eq!(loaded.launch.minecraft_root_path, absolute("mc-root"));
        assert!(loaded.launch.minecraft_root_path.is_absolute());

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    /// `load_sync` 与 `load` 走同一份解析 / 补路径逻辑（只是读文件同步）。
    #[test]
    fn load_sync_matches_load() {
        let dir = std::env::temp_dir().join(format!("dmcl-config-sync-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("dmcl.json");

        let mut config = Config::default();
        config.account.username = "tester".to_string();
        config.launch.minecraft_root_path = PathBuf::from("mc-root");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, serde_json::to_string_pretty(&config).unwrap()).unwrap();

        let loaded = Config::load_sync(Some(&path));
        assert!(loaded.error.is_none());
        assert_eq!(loaded.config.account.username, "tester");
        assert_eq!(
            loaded.config.launch.minecraft_root_path,
            absolute("mc-root")
        );

        // 文件不存在 ⇒ 默认值（而不是报错 / 警告）
        let missing = Config::load_sync(Some(&dir.join("missing.json")));
        assert!(missing.error.is_none());
        assert_eq!(missing.config.account.username, "player");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 坏配置项走 `load_sync` 时会被包进 `LoadedConfig::error`，配置本身仍可用。
    #[test]
    fn load_reports_problems_through_the_error_field() {
        let dir = std::env::temp_dir().join(format!("dmcl-config-bad-{}", std::process::id()));
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
}
