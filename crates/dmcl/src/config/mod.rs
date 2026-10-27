//! 配置。
//!
//! 解析与合并全交给 `serde`：嵌套结构体直接对应嵌套 JSON，`#[serde(default)]`
//! 表示「缺键保留默认值」。加载时逐项容错：不合法的配置项回退成默认值并汇总成
//! [`ConfigError`]，缺键仍然静默用默认值。
//!
//! **没有全局 `config` 变量**：[`Config`] 就是一个普通值，由前端（CLI / GUI）持有并传下去；
//! 各业务 crate 都不知道配置文件的存在（见各自的 `*Options`）。
//!
//! 全局配置分为 `launch` / `accounts` / `javas` / `minecraft_roots` / `minecraft_root` /
//! `download` 六部分；每个实例还可以在
//! `<minecraft_root>/versions/<version_name>/dmcl.json` 里写一个只含 `launch` 的覆盖层。
//!
//! ```json
//! {
//!   "launch": { "java": "autoselect", "account": "player", … },
//!   "accounts": { "player": { "type": "offline", "username": "player" } },
//!   "javas": { "jdk-17": "/usr/lib/jvm/java-17/bin/java" },
//!   "minecraft_roots": {
//!     "当前目录": "${CWD}/.minecraft",
//!     "官方启动器": "…"
//!   },
//!   "minecraft_root": "当前目录",
//!   "download": { "max_tasks": 64, "skip_music": false, … }
//! }
//! ```
//!
//! 动态表（`accounts` / `javas` / `minecraft_roots`）用 [`IndexMap`] 保存，顺序与配置文件一致。

mod account;
mod download;
mod launch;
mod root;

pub use account::AccountConfig;
pub use download::DownloadConfig;
pub use launch::{instance_config_path, LaunchConfig, LoadedInstanceConfig};

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use dmcl_download::{DownloadOptions, DEFAULT_USER_AGENT};
use dmcl_install::{InstallOptions, JavaProvider};
use dmcl_launch::{
    select_java, JavaChoice, JavaConfigs, LaunchOption, LAUNCHER_NAME, LAUNCHER_VERSION,
};
use dmcl_mcmeta::layout::absolute;
use dmcl_mcmeta::source::DownloadSource;
use indexmap::IndexMap;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// 1 MiB。
const MIB: u64 = 1024 * 1024;

/// 根目录路径里表示「当前工作目录」的占位符。
///
/// 它在取用根目录时展开，配置里始终保留原文，因此保存配置不会把当前目录写死。
pub const CWD_PLACEHOLDER: &str = "${CWD}";

/// 内置根目录「当前目录」的名字。
const CURRENT_ROOT_NAME: &str = "当前目录";

/// 内置根目录「官方启动器」的名字。
const OFFICIAL_ROOT_NAME: &str = "官方启动器";

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
            accounts: default_accounts(),
            javas: JavaConfigs::new(),
            minecraft_roots: default_roots(),
            minecraft_root: CURRENT_ROOT_NAME.to_string(),
            download: DownloadConfig::default(),
        }
    }
}

fn default_accounts() -> IndexMap<String, AccountConfig> {
    IndexMap::from([("player".to_string(), AccountConfig::default())])
}

fn default_roots() -> IndexMap<String, PathBuf> {
    IndexMap::from([
        (
            CURRENT_ROOT_NAME.to_string(),
            PathBuf::from(format!("{CWD_PLACEHOLDER}/.minecraft")),
        ),
        (OFFICIAL_ROOT_NAME.to_string(), official_launcher_root()),
    ])
}

/// 官方启动器存放 Minecraft 的目录。
fn official_launcher_root() -> PathBuf {
    #[cfg(target_os = "windows")]
    let dir = dirs::data_dir().map(|base| base.join(".minecraft"));
    #[cfg(target_os = "macos")]
    let dir = dirs::data_dir().map(|base| base.join("minecraft"));
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let dir = dirs::home_dir().map(|home| home.join(".minecraft"));
    dir.unwrap_or_else(|| PathBuf::from(".minecraft"))
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
            match section.as_str() {
                "launch" => {
                    root["launch"] = parse_section::<LaunchConfig>(
                        "launch",
                        body,
                        &root["launch"],
                        &mut warnings,
                    );
                }
                "download" => {
                    root["download"] = parse_section::<DownloadConfig>(
                        "download",
                        body,
                        &root["download"],
                        &mut warnings,
                    );
                }
                "accounts" => {
                    let fallback = default_accounts();
                    let table = parse_table("accounts", body, fallback, &mut warnings);
                    root["accounts"] = serde_json::to_value(table).unwrap_or_default();
                }
                "javas" => {
                    let table = parse_table("javas", body, JavaConfigs::new(), &mut warnings);
                    root["javas"] = serde_json::to_value(table).unwrap_or_default();
                }
                "minecraft_roots" => {
                    let roots = parse_roots(body, &mut warnings);
                    root["minecraft_roots"] = serde_json::to_value(roots).unwrap_or_default();
                }
                "minecraft_root" => {
                    if body.is_string() {
                        root["minecraft_root"] = body;
                    } else {
                        warnings.push(ConfigWarning {
                            message: format!(
                                "配置项「minecraft_root」使用默认值 {}",
                                root["minecraft_root"]
                            ),
                            detail: "值不合法：需要字符串".to_string(),
                        });
                    }
                }
                _ => {
                    warnings.push(ConfigWarning {
                        message: format!("配置段「{section}」已忽略"),
                        detail: "未知配置段".to_string(),
                    });
                }
            }
        }

        let mut config = match serde_json::from_value::<Self>(serde_json::Value::Object(root)) {
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
        let mut warnings = Vec::new();

        if self.minecraft_roots.is_empty() {
            self.minecraft_roots = default_roots();
            warnings.push(ConfigWarning {
                message: "配置项「minecraft_roots」为空，已加入默认根目录".to_string(),
                detail: format!(
                    "默认根目录为 {CURRENT_ROOT_NAME} -> {CWD_PLACEHOLDER}/.minecraft、\
                     {OFFICIAL_ROOT_NAME} -> {}",
                    official_launcher_root().display()
                ),
            });
        }

        if !self.minecraft_roots.contains_key(&self.minecraft_root) {
            let fallback = if self.minecraft_roots.contains_key(CURRENT_ROOT_NAME) {
                CURRENT_ROOT_NAME.to_string()
            } else {
                self.minecraft_roots
                    .keys()
                    .next()
                    .cloned()
                    .unwrap_or_default()
            };
            warnings.push(ConfigWarning {
                message: format!(
                    "配置项「minecraft_root」指向的根目录不存在：{}，改用「{fallback}」",
                    self.minecraft_root
                ),
                detail: "请编辑配置选择已有根目录".to_string(),
            });
            self.minecraft_root = fallback;
        }

        warnings.extend(validate_launch_choices(
            &mut self.launch,
            &self.accounts,
            &self.javas,
        ));
        warnings
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

    /// 按 `launch.account` 造一个账户。
    ///
    /// 没有选择账户（`null`）或指向不存在的账户时返回错误，不静默使用默认账户。
    pub fn selected_account(&self, launch: &LaunchConfig) -> Result<Arc<dyn dmcl_launch::Account>> {
        let name = launch
            .account
            .as_deref()
            .ok_or_else(|| anyhow!("实例没有选择账户（launch.account 为 null），请编辑配置"))?;
        let account = self
            .accounts
            .get(name)
            .ok_or_else(|| anyhow!("配置项「launch.account」指向的账号不存在：{name}"))?;
        match account {
            AccountConfig::Offline { username } => {
                Ok(Arc::new(dmcl_launch::OfflineAccount::new(username.clone())))
            }
        }
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
            .with_java(Arc::new(ConfigJava {
                choice: launch.java.clone(),
                configs: self.javas.clone(),
            })))
    }

    /// 配置 + 传入的启动配置 → [`LaunchOption`]。
    pub fn launch_option(
        &self,
        launch: &LaunchConfig,
        version_name: &str,
        root: &Path,
    ) -> Result<LaunchOption> {
        let account = self.selected_account(launch)?;
        let mut option = LaunchOption::new(account, root.to_path_buf(), version_name)
            .with_java(launch.java.clone(), self.javas.clone())
            .with_local_layout(self.download.skip_music);
        option.window_width = launch.window_width;
        option.window_height = launch.window_height;
        option.min_memory = launch.min_memory;
        option.max_memory = launch.max_memory;
        option.custom_info = launch.custom_info.clone();
        option.additional_jvm = launch
            .additional_jvm
            .split_whitespace()
            .map(str::to_string)
            .collect();
        option.additional_game = launch
            .additional_game
            .split_whitespace()
            .map(str::to_string)
            .collect();
        Ok(option)
    }
}

/// 账户 / Java 选择项回退；全局配置和实例配置共用。
pub(super) fn validate_launch_choices(
    launch: &mut LaunchConfig,
    accounts: &IndexMap<String, AccountConfig>,
    javas: &JavaConfigs,
) -> Vec<ConfigWarning> {
    let mut warnings = Vec::new();

    if let Some(name) = launch.account.clone() {
        if !accounts.contains_key(&name) {
            launch.account = None;
            warnings.push(ConfigWarning {
                message: format!("配置项「launch.account」指向的账号不存在：{name}，已改为 null"),
                detail: "请编辑配置选择已有账号".to_string(),
            });
        }
    }

    if let JavaChoice::Named(name) = &launch.java {
        if !javas.contains_key(name) {
            let name = name.clone();
            launch.java = JavaChoice::Auto;
            warnings.push(ConfigWarning {
                message: format!(
                    "配置项「launch.java」指向的 Java 不存在：{name}，已改为 autoselect"
                ),
                detail: "请编辑配置选择已有 Java".to_string(),
            });
        }
    }

    warnings
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

/// 解析根目录对象：名字 → 路径，条目逐个校验，坏条目丢弃。
fn parse_roots(
    body: serde_json::Value,
    warnings: &mut Vec<ConfigWarning>,
) -> IndexMap<String, PathBuf> {
    let serde_json::Value::Object(entries) = body else {
        warnings.push(ConfigWarning {
            message: "配置段「minecraft_roots」使用默认值".to_string(),
            detail: "该段不是 JSON 对象".to_string(),
        });
        return default_roots();
    };
    let mut roots = IndexMap::with_capacity(entries.len());
    for (name, raw) in entries {
        match serde_json::from_value::<PathBuf>(raw) {
            Ok(path) => {
                roots.insert(name, path);
            }
            Err(err) => warnings.push(ConfigWarning {
                message: format!("配置项「minecraft_roots.{name}」已忽略"),
                detail: format!("值不合法：{err}"),
            }),
        }
    }
    roots
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

    fn temp_dir(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("dmcl-config-{label}-{}", std::process::id()))
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
            "javas": { "jdk": "/usr/lib/jvm/java-17/bin/java" },
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
            Some(&PathBuf::from("/usr/lib/jvm/java-17/bin/java"))
        );
        assert_eq!(config.root_path("test").unwrap(), absolute("/mc"));
        assert_eq!(config.minecraft_root, "test");
        assert_eq!(config.download.mirror, MirrorPolicy::MirrorFirst);
        assert!(config.download.skip_music);
        // 缺键保留默认值
        assert_eq!(config.launch.window_height, 480);
        assert_eq!(config.launch.min_memory, 512);
        assert_eq!(config.download.max_tasks, 64);
    }

    #[test]
    fn defaults_include_a_player_account_two_roots_and_launch() {
        let config = Config::default();
        assert_eq!(
            config.accounts.get("player"),
            Some(&AccountConfig::Offline {
                username: "player".to_string()
            })
        );
        assert_eq!(
            config
                .roots()
                .into_iter()
                .map(|(name, _)| name)
                .collect::<Vec<_>>(),
            vec![
                CURRENT_ROOT_NAME.to_string(),
                OFFICIAL_ROOT_NAME.to_string()
            ]
        );
        assert_eq!(config.minecraft_root, CURRENT_ROOT_NAME);
        assert_eq!(config.default_root_path().unwrap(), absolute(".minecraft"));
        assert_eq!(
            config.minecraft_roots.get(CURRENT_ROOT_NAME),
            Some(&PathBuf::from(format!("{CWD_PLACEHOLDER}/.minecraft")))
        );
        assert_eq!(config.launch.account.as_deref(), Some("player"));
        assert!(config.launch.java.is_auto());
        assert_eq!(config.launch.custom_info, LAUNCHER_NAME);
        assert_eq!(config.download.mirror, MirrorPolicy::OfficialFirst);
        assert!(!config.download.skip_music);
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
        assert_eq!(config.launch.max_memory, 2048);
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
    fn dynamic_tables_replace_defaults_and_drop_bad_entries() {
        let (config, warnings) = Config::parse(
            r#"{
                "launch": { "account": "steve" },
                "accounts": {
                    "steve": { "type": "offline", "username": "Steve" },
                    "missing_name": { "type": "offline" },
                    "bad_type": { "type": "online", "username": "x" }
                },
                "javas": { "good": "/usr/bin/java", "bad": 1 },
                "minecraft_roots": {
                    "good": "/mc",
                    "bad": 1
                },
                "minecraft_root": "good"
            }"#,
        );
        assert!(config.accounts.contains_key("steve"));
        assert!(!config.accounts.contains_key("player"));
        assert!(!config.accounts.contains_key("missing_name"));
        assert!(!config.accounts.contains_key("bad_type"));
        assert!(config.javas.contains_key("good"));
        assert!(!config.javas.contains_key("bad"));
        assert!(config.root_path("good").is_ok());
        assert!(config.root_path("bad").is_err());
        for key in [
            "accounts.missing_name",
            "accounts.bad_type",
            "javas.bad",
            "minecraft_roots.bad",
        ] {
            assert!(
                warnings.iter().any(|warning| warning.message.contains(key)),
                "缺少 {key} 的警告: {warnings:?}"
            );
        }
    }

    #[test]
    fn invalid_minecraft_root_falls_back_to_default_or_first_entry() {
        let (config, warnings) = Config::parse(
            r#"{
                "minecraft_roots": { "a": "/a", "b": "/b" },
                "minecraft_root": "nope"
            }"#,
        );
        assert_eq!(config.minecraft_root, "a");
        assert_eq!(config.root_path("a").unwrap(), absolute("/a"));
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].message.contains("minecraft_root"));
    }

    #[test]
    fn roots_keep_the_file_order() {
        let text = r#"{
            "minecraft_roots": { "b": "/b", "a": "/a" },
            "minecraft_root": "a"
        }"#;
        let (config, warnings) = Config::parse(text);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(
            config
                .roots()
                .into_iter()
                .map(|(name, _)| name)
                .collect::<Vec<_>>(),
            vec!["b".to_string(), "a".to_string()]
        );

        let saved = serde_json::to_string_pretty(&config).unwrap();
        let roots_at = saved.find("\"minecraft_roots\"").unwrap();
        let first = saved[roots_at..].find("\"b\"").unwrap();
        let second = saved[roots_at..].find("\"a\"").unwrap();
        assert!(first < second, "{saved}");

        let (reloaded, warnings) = Config::parse(&saved);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(
            reloaded
                .roots()
                .into_iter()
                .map(|(name, _)| name)
                .collect::<Vec<_>>(),
            vec!["b".to_string(), "a".to_string()]
        );
    }

    #[test]
    fn dynamic_tables_keep_the_file_order() {
        let text = r#"{
            "launch": { "account": "a" },
            "accounts": {
                "b": { "type": "offline", "username": "B" },
                "a": { "type": "offline", "username": "A" }
            },
            "javas": { "b": "/b/java", "a": "/a/java" }
        }"#;
        let (config, warnings) = Config::parse(text);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(
            config
                .accounts
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec!["b", "a"]
        );
        assert_eq!(
            config.javas.keys().map(String::as_str).collect::<Vec<_>>(),
            vec!["b", "a"]
        );

        let saved = serde_json::to_string_pretty(&config).unwrap();
        let (reloaded, warnings) = Config::parse(&saved);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(
            reloaded
                .accounts
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec!["b", "a"]
        );
        assert_eq!(
            reloaded
                .javas
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec!["b", "a"]
        );
    }

    #[test]
    fn select_minecraft_root_requires_an_existing_name() {
        let mut config = Config::default();
        config
            .minecraft_roots
            .insert("test".to_string(), PathBuf::from("/mc"));
        config.select_minecraft_root("test").unwrap();
        assert_eq!(config.minecraft_root, "test");
        assert_eq!(config.default_root_path().unwrap(), absolute("/mc"));

        let err = config.select_minecraft_root("nope").unwrap_err();
        assert!(err.to_string().contains("根目录不存在：nope"));
        assert_eq!(config.minecraft_root, "test");
    }

    #[test]
    fn named_roots_resolve_and_default_can_be_selected() {
        let mut config = Config::default();
        config
            .minecraft_roots
            .insert("test".to_string(), PathBuf::from("/mc"));
        config.select_minecraft_root("test").unwrap();
        assert_eq!(config.default_root_name().unwrap(), "test");
        assert_eq!(config.default_root_path().unwrap(), absolute("/mc"));
        assert_eq!(
            config.root_path(CURRENT_ROOT_NAME).unwrap(),
            absolute(".minecraft")
        );
        assert_eq!(
            config
                .roots()
                .into_iter()
                .map(|(name, _)| name)
                .collect::<Vec<_>>(),
            vec![
                CURRENT_ROOT_NAME.to_string(),
                OFFICIAL_ROOT_NAME.to_string(),
                "test".to_string()
            ]
        );
        assert!(config.root_path("nope").is_err());
    }

    #[test]
    fn empty_root_map_gets_the_default_roots() {
        let (config, warnings) = Config::parse(r#"{"minecraft_roots": {}}"#);
        assert_eq!(
            config
                .roots()
                .into_iter()
                .map(|(name, _)| name)
                .collect::<Vec<_>>(),
            vec![
                CURRENT_ROOT_NAME.to_string(),
                OFFICIAL_ROOT_NAME.to_string()
            ]
        );
        assert_eq!(config.minecraft_root, CURRENT_ROOT_NAME);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].message.contains("minecraft_roots"));
    }

    #[test]
    fn empty_account_table_stays_empty_and_clears_the_selection() {
        let (config, warnings) = Config::parse(r#"{"accounts": {}}"#);
        assert!(config.accounts.is_empty());
        assert_eq!(config.launch.account, None);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].message.contains("launch.account"));
    }

    #[test]
    fn a_missing_named_java_falls_back_to_autoselect() {
        let (config, warnings) = Config::parse(r#"{"launch": {"java": "nope"}}"#);
        assert!(config.launch.java.is_auto());
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].message.contains("launch.java"));
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

    #[test]
    fn the_cwd_placeholder_is_resolved_but_saved_as_is() {
        let mut config = Config::default();
        config.minecraft_roots.insert(
            "minecraft".to_string(),
            PathBuf::from(format!("{}/mc", CWD_PLACEHOLDER)),
        );

        let text = serde_json::to_string_pretty(&config).unwrap();
        assert!(text.contains("\"${CWD}/.minecraft\""), "{text}");
        assert!(text.contains("\"${CWD}/mc\""), "{text}");

        let (loaded, warnings) = Config::parse(&text);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(loaded.default_root_path().unwrap(), absolute(".minecraft"));
        assert_eq!(loaded.root_path("minecraft").unwrap(), absolute("mc"));
    }

    #[test]
    fn the_official_root_uses_the_platform_launcher_directory() {
        let root = Config::default().root_path(OFFICIAL_ROOT_NAME).unwrap();
        #[cfg(target_os = "windows")]
        assert_eq!(root, dirs::data_dir().unwrap().join(".minecraft"));
        #[cfg(target_os = "macos")]
        assert_eq!(root, dirs::data_dir().unwrap().join("minecraft"));
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        assert_eq!(root, dirs::home_dir().unwrap().join(".minecraft"));
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

    #[tokio::test]
    async fn a_missing_instance_file_uses_the_global_launch() {
        let dir = temp_dir("instance-missing");
        let _ = std::fs::remove_dir_all(&dir);
        let mut config = Config::default();
        config
            .minecraft_roots
            .insert("test".to_string(), dir.join("mc"));
        config.minecraft_root = "test".to_string();
        config.launch.max_memory = 3000;

        let root = dir.join("mc");
        let loaded = config.load_instance(&root, "1.20.1").await;
        assert!(loaded.error.is_none());
        assert_eq!(loaded.launch.max_memory, 3000);
        assert!(!instance_config_path(&root, "1.20.1").exists());
    }

    #[tokio::test]
    async fn instance_fields_override_only_what_they_mention() {
        let dir = temp_dir("instance-partial");
        let _ = std::fs::remove_dir_all(&dir);
        let root = dir.join("mc");
        let mut config = Config::default();
        config
            .minecraft_roots
            .insert("test".to_string(), root.clone());
        config.minecraft_root = "test".to_string();
        config.launch.max_memory = 3000;

        let path = instance_config_path(&root, "1.20.1");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let text = r#"{"launch": {"max_memory": 4096, "account": null}}"#;
        std::fs::write(&path, text).unwrap();

        let loaded = config.load_instance(&root, "1.20.1").await;
        assert!(loaded.error.is_none(), "{:?}", loaded.error);
        assert_eq!(loaded.launch.max_memory, 4096);
        assert_eq!(loaded.launch.window_width, 854);
        assert_eq!(loaded.launch.account, None);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);

        let sync = config.load_instance_sync(&root, "1.20.1");
        assert_eq!(sync.launch.max_memory, 4096);
        assert_eq!(sync.launch.account, None);
        assert!(sync.error.is_none());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn invalid_instance_fields_fall_back_to_the_global_values() {
        let dir = temp_dir("instance-invalid");
        let _ = std::fs::remove_dir_all(&dir);
        let root = dir.join("mc");
        let mut config = Config::default();
        config
            .minecraft_roots
            .insert("test".to_string(), root.clone());
        config.minecraft_root = "test".to_string();
        config.launch.max_memory = 3000;
        config.launch.window_width = 1000;

        let path = instance_config_path(&root, "1.20.1");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            r#"{
                "launch": {
                    "max_memory": -1,
                    "window_width": 1200,
                    "unknown": true
                },
                "download": {}
            }"#,
        )
        .unwrap();

        let loaded = config.load_instance(&root, "1.20.1").await;
        assert_eq!(loaded.launch.max_memory, 3000);
        assert_eq!(loaded.launch.window_width, 1200);
        let error = loaded.error.as_ref().expect("应当有警告");
        assert!(error
            .warnings
            .iter()
            .any(|warning| warning.message.contains("launch.max_memory")));
        assert!(error
            .warnings
            .iter()
            .any(|warning| warning.message.contains("launch.unknown")));
        assert!(error
            .warnings
            .iter()
            .any(|warning| warning.message.contains("download")));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn instance_account_and_java_selections_are_validated_after_merging() {
        let dir = temp_dir("instance-selection");
        let _ = std::fs::remove_dir_all(&dir);
        let root = dir.join("mc");
        let mut config = Config::default();
        config
            .minecraft_roots
            .insert("test".to_string(), root.clone());
        config.minecraft_root = "test".to_string();

        let path = instance_config_path(&root, "1.20.1");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            r#"{"launch": {"account": "missing", "java": "missing"}}"#,
        )
        .unwrap();

        let loaded = config.load_instance(&root, "1.20.1").await;
        assert_eq!(loaded.launch.account, None);
        assert!(loaded.launch.java.is_auto());
        let error = loaded.error.as_ref().expect("应当有警告");
        assert_eq!(error.warnings.len(), 2);
        assert!(config
            .selected_account(&loaded.launch)
            .err()
            .expect("应当报错")
            .to_string()
            .contains("没有选择账户"));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn selected_account_requires_an_explicit_valid_selection() {
        let config = Config::default();
        let mut launch = config.launch.clone();
        launch.account = None;
        assert!(config.selected_account(&launch).is_err());

        launch.account = Some("nope".to_string());
        let err = config.selected_account(&launch).err().expect("应当报错");
        assert!(err.to_string().contains("账号不存在：nope"));

        launch.account = Some("player".to_string());
        assert_eq!(
            config.selected_account(&launch).unwrap().get_name(),
            "player"
        );
    }

    #[test]
    fn the_config_maps_onto_the_crate_options() {
        let dir = temp_dir("options");
        let _ = std::fs::remove_dir_all(&dir);
        let mut config = Config::default();
        config
            .minecraft_roots
            .insert("test".to_string(), dir.join("mc"));
        config.minecraft_root = "test".to_string();
        config.download.max_tasks = 8;
        config.download.chunk_threshold_mib = 2;
        config.download.chunk_size_mib = 1;
        config.download.max_chunks_per_file = 3;
        config.download.max_attempts = 5;
        config.download.skip_music = true;
        config.accounts.insert(
            "steve".to_string(),
            AccountConfig::Offline {
                username: "Steve".to_string(),
            },
        );
        config.launch.account = Some("steve".to_string());
        config.launch.window_width = 1920;
        config.launch.min_memory = 1024;
        config.launch.max_memory = 4096;
        config.launch.custom_info = "test".to_string();
        config.launch.additional_jvm = "-Da=1  -Db=2".to_string();
        config.launch.additional_game = "--foo bar".to_string();
        config
            .javas
            .insert("jdk".to_string(), PathBuf::from("/nonexistent/bin/java"));
        config.launch.java = JavaChoice::Named("jdk".to_string());

        let download = config.download_options();
        assert_eq!(download.max_connections, 8);
        assert_eq!(download.chunk_threshold, 2 * MIB);
        assert_eq!(download.chunk_size, MIB);
        assert_eq!(download.max_chunks_per_file, 3);
        assert_eq!(download.max_attempts, 5);

        let root = dir.join("mc");
        let install = config
            .install_options(config.download_source(), &config.launch, &root)
            .unwrap();
        assert_eq!(install.root_path, dir.join("mc"));
        assert!(install.skip_music);
        assert_eq!(install.download.max_connections, 8);

        let option = config
            .launch_option(&config.launch, "1.20.1", &root)
            .unwrap();
        assert_eq!(option.root_path, dir.join("mc"));
        assert_eq!(option.window_width, 1920);
        assert_eq!(option.min_memory, 1024);
        assert_eq!(option.max_memory, 4096);
        assert_eq!(option.custom_info, "test");
        assert!(option.skip_music);
        assert!(option.additional_jvm.contains(&"-Da=1".to_string()));
        assert!(option.additional_jvm.contains(&"-Db=2".to_string()));
        assert_eq!(option.additional_game, vec!["--foo", "bar"]);
        assert_eq!(option.account.get_name(), "Steve");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
