//! 配置。
//!
//! 解析与合并全交给 `serde`：嵌套结构体直接对应嵌套 JSON，`#[serde(default)]`
//! 表示"缺键保留默认值"。
//!
//! **没有全局 `config` 变量**：[`Config`] 就是一个普通值，由 [`crate::cli::run`]
//! 持有并传下去；各业务 crate 都不知道配置文件的存在（见各自的 `*Options`）。
//!
//! ```json
//! {
//!   "launch":   { "java": "autoselect", "minecraft_root_path": "/…/.minecraft", … },
//!   "account":  { "online_mode": false, "username": "player" },
//!   "download": { "max_tasks": 64, "mirror": "official-first", … },
//!   "cli":      { "default_command": "prompt" }
//! }
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use dmcl_download::{DownloadOptions, DEFAULT_USER_AGENT};
use dmcl_install::{InstallOptions, JavaProvider};
use dmcl_launch::{
    select_java, JavaChoice, JavaConfigs, LaunchOption, LAUNCHER_NAME, LAUNCHER_VERSION,
};
use dmcl_mcmeta::layout::absolute;
use dmcl_mcmeta::source::{DownloadSource, MirrorPolicy};
use serde::{Deserialize, Serialize};

/// 1 MiB。
const MIB: u64 = 1024 * 1024;

/// `launch.*` 段。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LaunchConfig {
    /// 命名 java（名字 → 可执行文件），由 `dmcl java` 维护。
    pub java_configs: JavaConfigs,
    /// 用哪一个，或自动选择。
    pub java: JavaChoice,
    pub minecraft_root_path: PathBuf,
    /// 窗口尺寸（像素）—— 不可能为负。
    pub window_width: u32,
    pub window_height: u32,
    /// 堆大小（MiB）—— 不可能为负。
    pub min_memory: u32,
    pub max_memory: u32,
    pub custom_info: String,
    pub additional_jvm: String,
    pub additional_game: String,
}

impl Default for LaunchConfig {
    fn default() -> Self {
        Self {
            java_configs: BTreeMap::new(),
            java: JavaChoice::Auto,
            minecraft_root_path: PathBuf::from(".minecraft"),
            window_width: 854,
            window_height: 480,
            min_memory: 512,
            max_memory: 2048,
            custom_info: LAUNCHER_NAME.to_string(),
            // 历史默认：这两个 `-D` 是给 FML/OptiFine 兼容用的，从旧实现继承下来。
            // 以后应改成空 / 更中性的默认，或交给用户配置。
            additional_jvm:
                "-Dfml.ignoreInvalidMinecraftCertificates=True -Dfml.ignorePatchDiscrepancies=True"
                    .to_string(),
            additional_game: String::new(),
        }
    }
}

/// `account.*` 段。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AccountConfig {
    /// 仍未使用的配置项：账号目前只有离线实现。
    pub online_mode: bool,
    pub username: String,
}

impl Default for AccountConfig {
    fn default() -> Self {
        Self {
            online_mode: false,
            username: "player".to_string(),
        }
    }
}

/// `download.*` 段。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DownloadConfig {
    /// 同时进行的下载连接数上限。
    pub max_tasks: usize,
    /// 跳过 `*.ogg` 资源（音乐与音效）。
    pub i_dont_need_music: bool,
    /// 镜像使用策略（仅官方 / 官方优先 / 镜像优先）。
    pub mirror: MirrorPolicy,
    /// 超过这个大小（MiB）就分块下载。
    pub chunk_threshold_mib: u64,
    /// 每块大小（MiB）。
    pub chunk_size_mib: u64,
    /// 单个文件最多并发几块。
    pub max_chunks_per_file: usize,
    /// 单个文件最多尝试几次。
    pub max_attempts: u32,
}

impl Default for DownloadConfig {
    fn default() -> Self {
        Self {
            max_tasks: 64,
            i_dont_need_music: false,
            mirror: MirrorPolicy::default(),
            chunk_threshold_mib: 8,
            chunk_size_mib: 4,
            max_chunks_per_file: 8,
            max_attempts: 3,
        }
    }
}

/// `cli.*` 段。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CliConfig {
    pub default_command: String,
}

impl Default for CliConfig {
    fn default() -> Self {
        Self {
            default_command: "prompt".to_string(),
        }
    }
}

/// 整份配置文档。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub launch: LaunchConfig,
    pub account: AccountConfig,
    pub download: DownloadConfig,
    pub cli: CliConfig,
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
    pub async fn load(path: Option<&Path>) -> Result<Self> {
        let path = path.map(Path::to_path_buf).unwrap_or_else(Self::path);
        let mut config = match tokio::fs::read_to_string(&path).await {
            Ok(text) => serde_json::from_str(&text)
                .with_context(|| format!("无法解析配置文件 {}", path.display()))?,
            // 文件不存在就用默认值；其它读错误（权限等）要报出来
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Config::default(),
            Err(err) => {
                return Err(err).with_context(|| format!("无法读取配置文件 {}", path.display()))
            }
        };
        config.launch.minecraft_root_path = absolute(&config.launch.minecraft_root_path);
        Ok(config)
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

    /// 人类可读的配置转储（字段名与 `dmcl.json` 的键一致）。
    ///
    /// 字段清单由 serde 派生自动给出（见 [`Config::fields`]），不手写。
    pub fn show(&self) {
        // 只有标题行是给人看的说明；下面的键名刻意保持与 dmcl.json 一致
        println!("配置文件：{}\n", Self::path().display());
        match self.fields() {
            Ok(fields) => {
                for (key, value) in fields {
                    println!("{key} = {}", display_value(&value));
                }
            }
            Err(err) => eprintln!("无法展开配置：{err:#}"),
        }
    }

    /// 把配置展开成「点分键 → JSON 值」的列表（`launch.window_width` …）。
    ///
    /// 键名来自 serde 序列化，因此新增字段不用改这里。只展开两层：顶层是各段
    /// （`launch` / `account` / `download` / `cli`），段里的字段整体作为一个值
    /// （`launch.java_configs` 这种就按 JSON 编辑）。
    pub fn fields(&self) -> Result<Vec<(String, serde_json::Value)>> {
        let root = serde_json::to_value(self)?;
        let mut fields = Vec::new();
        let serde_json::Value::Object(sections) = root else {
            return Ok(fields);
        };
        for (section, body) in sections {
            match body {
                serde_json::Value::Object(entries) => {
                    for (field, value) in entries {
                        fields.push((format!("{section}.{field}"), value));
                    }
                }
                other => fields.push((section, other)),
            }
        }
        Ok(fields)
    }

    /// 按点分键写回一个 JSON 值。
    ///
    /// 写回后整体反序列化校验（`deny_unknown_fields`、类型等都在这道关卡上）；
    /// 校验失败时**不改动** `self`。
    pub fn set_field(&mut self, key: &str, value: serde_json::Value) -> Result<()> {
        let (section, field) = key
            .split_once('.')
            .ok_or_else(|| anyhow!("配置键格式应为 section.field，实际是 {key}"))?;
        let mut root = serde_json::to_value(&*self)?;
        let body = root
            .get_mut(section)
            .and_then(serde_json::Value::as_object_mut)
            .ok_or_else(|| anyhow!("没有这一段配置：{section}"))?;
        body.insert(field.to_string(), value);
        *self = serde_json::from_value(root).with_context(|| format!("{key} 的值不合法"))?;
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

/// 配置值在菜单 / `show` 里的**单行**呈现：标量按普通文本，对象 / 数组按紧凑 JSON。
pub fn display_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Null => String::new(),
        serde_json::Value::Object(_) | serde_json::Value::Array(_) => value.to_string(),
        other => other.to_string(),
    }
}

/// 对象 / 数组类型的值（结构化字段，**不支持**在交互编辑器里修改）。
pub fn is_json_value(value: &serde_json::Value) -> bool {
    matches!(
        value,
        serde_json::Value::Object(_) | serde_json::Value::Array(_)
    )
}

/// 把编辑后的文本解析成 JSON 值：类型按 `current` 推断（字符串不加引号、布尔写 true/false）。
///
/// 只处理标量；结构化值（对象 / 数组）不走这里。
pub fn parse_edited_value(text: &str, current: &serde_json::Value) -> Result<serde_json::Value> {
    match current {
        serde_json::Value::String(_) | serde_json::Value::Null => {
            Ok(serde_json::Value::String(text.to_string()))
        }
        serde_json::Value::Bool(_) => match text.trim() {
            "true" => Ok(serde_json::Value::Bool(true)),
            "false" => Ok(serde_json::Value::Bool(false)),
            other => Err(anyhow!("需要 true 或 false，实际是 {other:?}")),
        },
        serde_json::Value::Number(_) => {
            let trimmed = text.trim();
            if let Ok(int) = trimmed.parse::<i64>() {
                Ok(int.into())
            } else if let Ok(uint) = trimmed.parse::<u64>() {
                Ok(uint.into())
            } else if let Ok(float) = trimmed.parse::<f64>() {
                serde_json::Number::from_f64(float)
                    .map(serde_json::Value::Number)
                    .ok_or_else(|| anyhow!("不是合法的数字：{trimmed:?}"))
            } else {
                Err(anyhow!("需要一个数字，实际是 {trimmed:?}"))
            }
        }
        serde_json::Value::Object(_) | serde_json::Value::Array(_) => {
            Err(anyhow!("结构化配置项不能在这里修改"))
        }
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
        assert_eq!(config.cli.default_command, "prompt");
    }

    #[test]
    fn serialises_to_the_same_key_layout() {
        let value = serde_json::to_value(Config::default()).unwrap();
        assert!(value["launch"]["java"].is_string());
        assert!(value["launch"]["minecraft_root_path"].is_string());
        assert!(value["account"]["online_mode"].is_boolean());
        assert!(value["download"]["max_tasks"].is_number());
        assert!(value["download"]["mirror"].is_string());
        assert!(value["cli"]["default_command"].is_string());
    }

    #[test]
    fn empty_document_yields_all_defaults() {
        let config: Config = serde_json::from_str("{}").unwrap();
        assert!(config.launch.java.is_auto());
        assert_eq!(config.launch.custom_info, LAUNCHER_NAME);
        assert_eq!(config.download.mirror, MirrorPolicy::OfficialFirst);
    }

    #[test]
    fn negative_sizes_are_rejected_instead_of_wrapping() {
        // D 的 `int` 会接受这些值并一路传给 JVM
        assert!(serde_json::from_str::<Config>(r#"{"launch":{"window_width":-1}}"#).is_err());
        assert!(serde_json::from_str::<Config>(r#"{"launch":{"max_memory":-2048}}"#).is_err());
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

    /// 配置项清单由 serde 派生自动展开，不用手写；段里含的字段都在。
    #[test]
    fn fields_are_derived_from_serde() {
        let mut config = Config::default();
        config.launch.window_width = 1920;
        config
            .launch
            .java_configs
            .insert("jdk".to_string(), PathBuf::from("/usr/bin/java"));

        let fields = config.fields().unwrap();
        let keys: Vec<&str> = fields.iter().map(|(key, _)| key.as_str()).collect();
        for expected in [
            "launch.java",
            "launch.java_configs",
            "launch.minecraft_root_path",
            "launch.window_width",
            "account.username",
            "download.mirror",
            "cli.default_command",
        ] {
            assert!(keys.contains(&expected), "缺少 {expected}: {keys:?}");
        }
        let width = fields
            .iter()
            .find(|(key, _)| key == "launch.window_width")
            .map(|(_, value)| value.clone())
            .unwrap();
        assert_eq!(width, serde_json::json!(1920));
    }

    #[test]
    fn set_field_round_trips_and_rejects_invalid_values() {
        let mut config = Config::default();
        config
            .set_field("launch.window_width", serde_json::json!(1920))
            .unwrap();
        assert_eq!(config.launch.window_width, 1920);

        // 写回后整体反序列化校验：负数会被 `u32` 拒绝，且不改动原值
        assert!(config
            .set_field("launch.window_width", serde_json::json!(-1))
            .is_err());
        assert_eq!(config.launch.window_width, 1920);

        // 名字写错 / 段不存在都要报错
        assert!(config
            .set_field("launch.nope", serde_json::json!(1))
            .is_err());
        assert!(config.set_field("nope.a", serde_json::json!(1)).is_err());

        // java_configs 按 JSON 对象写回
        config
            .set_field(
                "launch.java_configs",
                serde_json::json!({ "jdk": "/usr/bin/java" }),
            )
            .unwrap();
        assert_eq!(
            config.launch.java_configs.get("jdk").unwrap(),
            &PathBuf::from("/usr/bin/java")
        );
    }

    #[test]
    fn edited_text_is_parsed_by_the_current_type() {
        use serde_json::json;
        assert_eq!(
            parse_edited_value("hello", &json!("x")).unwrap(),
            json!("hello")
        );
        assert_eq!(
            parse_edited_value("true", &json!(false)).unwrap(),
            json!(true)
        );
        assert_eq!(parse_edited_value(" 42 ", &json!(0)).unwrap(), json!(42));
        assert!(parse_edited_value("abc", &json!(0)).is_err());
        assert!(parse_edited_value("maybe", &json!(true)).is_err());
        // 结构化值不走这里（编辑器直接提示无法修改）
        assert!(parse_edited_value(r#"{"a":"b"}"#, &json!({})).is_err());
    }

    #[test]
    fn values_are_displayed_compactly() {
        use serde_json::json;
        assert_eq!(display_value(&json!("text")), "text");
        assert_eq!(display_value(&json!(854)), "854");
        assert_eq!(display_value(&json!(true)), "true");
        assert_eq!(display_value(&json!({"a": "b"})), r#"{"a":"b"}"#);
        assert!(is_json_value(&json!({})));
        assert!(!is_json_value(&json!("x")));
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

        let loaded = Config::load(Some(&path)).await.unwrap();
        assert_eq!(loaded.account.username, "tester");
        assert_eq!(loaded.download.mirror, MirrorPolicy::MirrorFirst);
        // 相对路径会被补成绝对路径（相对于进程当前目录）
        assert_eq!(loaded.launch.minecraft_root_path, absolute("mc-root"));
        assert!(loaded.launch.minecraft_root_path.is_absolute());

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }
}
