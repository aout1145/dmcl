//! 配置（原 `dmcl.config`）。
//!
//! 原实现用 D 的编译期反射（`__traits(allMembers, Config)` + `replaceFirst("_", ".")`）
//! 把扁平字段名映射成点分 JSON 键，并手搓解析与合并；这里全部交给 `serde`：
//! 嵌套结构体直接对应嵌套 JSON（正好就是点分键展开后的样子），
//! `#[serde(default)]` 复现"缺键保留默认值"的语义。配置文件的键名与原版兼容。
//!
//! 与原实现的另一个区别：**不再有全局 `config` 变量**。
//! [`Config`] 就是一个普通值，由 [`crate::cli::run`] 持有并传下去；
//! 各业务 crate 更是一点都不知道配置文件的存在（见各自的 `*Options`）。
//!
//! ```json
//! {
//!   "launch":   { "java": "autoselect", "minecraft_root_path": "/…/.minecraft", … },
//!   "account":  { "online_mode": false, "username": "player" },
//!   "download": { "max_tasks": 64, "mirror": "official", … },
//!   "cli":      { "default_command": "prompt" }
//! }
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use dmcl_download::{DownloadOptions, DEFAULT_USER_AGENT};
use dmcl_install::{InstallOptions, JavaProvider};
use dmcl_launch::{
    select_java, JavaChoice, JavaConfigs, LaunchOption, LAUNCHER_NAME, LAUNCHER_VERSION,
};
use dmcl_mcmeta::source::{DownloadSource, Mirror};
use path_clean::PathClean;
use serde::{Deserialize, Serialize};

/// 1 MiB。
const MIB: u64 = 1024 * 1024;

/// `launch.*` 段。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LaunchConfig {
    /// 命名 java（名字 → 可执行文件），由 `search-java` 填充。
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
    /// natives 分类器里 `${arch}` 的取值；留空表示按本机位宽。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub natives_arch: Option<u32>,
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
            additional_jvm:
                "-Dfml.ignoreInvalidMinecraftCertificates=True -Dfml.ignorePatchDiscrepancies=True"
                    .to_string(),
            additional_game: String::new(),
            natives_arch: None,
        }
    }
}

/// `account.*` 段。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
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
#[serde(default)]
pub struct DownloadConfig {
    /// 同时进行的下载连接数上限。
    pub max_tasks: usize,
    /// 跳过 `*.ogg` 资源（音乐与音效）。
    pub i_dont_need_music: bool,
    pub mirror: Mirror,
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
            mirror: Mirror::default(),
            chunk_threshold_mib: 8,
            chunk_size_mib: 4,
            max_chunks_per_file: 8,
            max_attempts: 3,
        }
    }
}

/// `cli.*` 段。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
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
#[serde(default)]
pub struct Config {
    pub launch: LaunchConfig,
    pub account: AccountConfig,
    pub download: DownloadConfig,
    pub cli: CliConfig,
}

impl Config {
    /// `getConfigPath` —— `<平台配置目录>/dmcl.json`。
    ///
    /// 基准目录来自 [`dirs`]，不再手挑 `%APPDATA%` / `$HOME/.config`：
    ///
    /// * Linux —— `$XDG_CONFIG_HOME` 或 `~/.config` → 与原版同一个文件
    /// * Windows —— `%APPDATA%` → 与原版同一个文件
    /// * macOS —— `~/Library/Application Support` → **原来是 `~/.config`**，
    ///   这是唯一一个文件位置变了的平台
    pub fn path() -> PathBuf {
        match dirs::config_dir() {
            Some(base) => absolute(base.join("dmcl.json")),
            None => absolute("dmcl.json"),
        }
    }

    /// `readConfig` —— 读配置（文件不存在就用默认值），并把 `.minecraft` 路径补成绝对路径。
    pub async fn load(path: Option<&Path>) -> Result<Self> {
        let path = path.map(Path::to_path_buf).unwrap_or_else(Self::path);
        let mut config = match tokio::fs::read_to_string(&path).await {
            Ok(text) => serde_json::from_str(&text)
                .with_context(|| format!("无法解析配置文件 {}", path.display()))?,
            Err(_) => Config::default(),
        };
        config.launch.minecraft_root_path = absolute(&config.launch.minecraft_root_path);
        Ok(config)
    }

    /// `saveConfig` —— 持久化整份配置。
    ///
    /// 原实现只把指定字段合并进已有文件；因为**所有**字段都会被读出来，整体序列化是等价的。
    pub async fn save(&self) -> Result<()> {
        let path = Self::path();
        let text = serde_json::to_string_pretty(self)?;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.ok();
        }
        tokio::fs::write(&path, text)
            .await
            .with_context(|| format!("无法写入配置文件 {}", path.display()))?;
        Ok(())
    }

    /// `showConfig` —— 人类可读的转储（字段名与 D 版一致）。
    pub fn show(&self) {
        // 下面这些字段名刻意保持与 dmcl.json 的键一致（它们是配置键，不是文案），
        // 只有标题行是给人看的说明。
        println!("配置文件：{}\n", Self::path().display());
        println!("launch.java = {}", self.launch.java);
        println!(
            "launch.java_configs = {{{}}}",
            self.launch
                .java_configs
                .iter()
                .map(|(name, path)| format!("{name}: {}", path.display()))
                .collect::<Vec<_>>()
                .join(", ")
        );
        println!(
            "launch.minecraft_root_path = {}",
            self.launch.minecraft_root_path.display()
        );
        println!("launch.window_width = {}", self.launch.window_width);
        println!("launch.window_height = {}", self.launch.window_height);
        println!("launch.min_memory = {}", self.launch.min_memory);
        println!("launch.max_memory = {}", self.launch.max_memory);
        println!("launch.custom_info = {}", self.launch.custom_info);
        println!("launch.additional_jvm = {}", self.launch.additional_jvm);
        println!("launch.additional_game = {}", self.launch.additional_game);
        println!(
            "launch.natives_arch = {}",
            self.launch
                .natives_arch
                .map(|arch| arch.to_string())
                .unwrap_or_else(|| "auto".to_string())
        );
        println!("account.online_mode = {}", self.account.online_mode);
        println!("account.username = {}", self.account.username);
        println!("download.max_tasks = {}", self.download.max_tasks);
        println!(
            "download.i_dont_need_music = {}",
            self.download.i_dont_need_music
        );
        println!("download.mirror = {}", self.download.mirror);
        println!(
            "download.chunk_threshold_mib = {}",
            self.download.chunk_threshold_mib
        );
        println!("download.chunk_size_mib = {}", self.download.chunk_size_mib);
        println!(
            "download.max_chunks_per_file = {}",
            self.download.max_chunks_per_file
        );
        println!("download.max_attempts = {}", self.download.max_attempts);
        println!("cli.default_command = {}", self.cli.default_command);
    }

    /// 按配置造一个下载源（镜像 + User-Agent）。
    pub fn download_source(&self) -> DownloadSource {
        DownloadSource::new(self.download.mirror)
            .with_user_agent(format!("{LAUNCHER_NAME}/{LAUNCHER_VERSION}"))
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
        let mut options = InstallOptions::new(self.launch.minecraft_root_path.clone(), source)
            .with_download(self.download_options())
            .with_skip_music(self.download.i_dont_need_music)
            .with_java(Arc::new(ConfigJava {
                choice: self.launch.java.clone(),
                configs: self.launch.java_configs.clone(),
            }));
        if let Some(arch) = self.launch.natives_arch {
            options = options.with_natives_arch(arch);
        }
        options
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
        .with_local_layout(
            self.download.i_dont_need_music,
            self.launch
                .natives_arch
                .unwrap_or_else(dmcl_install::options::default_natives_arch),
        );
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

/// 把路径变成绝对路径并归一化。
pub fn absolute(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    absolute.clean()
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
            "download": { "mirror": "bmclapi" }
        }"#;
        let config: Config = serde_json::from_str(text).unwrap();
        assert_eq!(config.launch.java, JavaChoice::Named("myjdk".to_string()));
        assert_eq!(config.launch.window_width, 1280);
        assert_eq!(
            config.launch.java_configs.get("jdk").unwrap(),
            &PathBuf::from("/usr/bin/java")
        );
        assert_eq!(config.download.mirror, Mirror::Bmclapi);
        // 缺键保留默认值，与原来的读取逻辑一致
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
        // 没设置时不写出 natives_arch
        assert!(value["launch"].get("natives_arch").is_none());
    }

    #[test]
    fn empty_document_yields_all_defaults() {
        let config: Config = serde_json::from_str("{}").unwrap();
        assert!(config.launch.java.is_auto());
        assert_eq!(config.launch.custom_info, LAUNCHER_NAME);
        assert_eq!(config.download.mirror, Mirror::Official);
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
        config.launch.natives_arch = Some(32);
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
        assert_eq!(install.natives_arch, 32);
        assert_eq!(install.download.max_connections, 8);

        let option = config.launch_option(config.account(), "1.20.1");
        assert_eq!(option.root_path, PathBuf::from("/mc"));
        assert_eq!(option.window_width, 1920);
        assert_eq!(option.natives_arch, 32);
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
        config.download.mirror = Mirror::Bmclapi;
        config.launch.minecraft_root_path = PathBuf::from("/tmp/mc-root");

        // 手动写到指定路径（`save` 用的是平台配置目录，测试里不碰它）
        tokio::fs::create_dir_all(&dir).await.unwrap();
        tokio::fs::write(&path, serde_json::to_string_pretty(&config).unwrap())
            .await
            .unwrap();

        let loaded = Config::load(Some(&path)).await.unwrap();
        assert_eq!(loaded.account.username, "tester");
        assert_eq!(loaded.download.mirror, Mirror::Bmclapi);
        assert_eq!(
            loaded.launch.minecraft_root_path,
            PathBuf::from("/tmp/mc-root")
        );
        // 相对路径会被补成绝对路径
        assert!(loaded.launch.minecraft_root_path.is_absolute());

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }
}
