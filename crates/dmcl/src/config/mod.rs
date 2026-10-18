//! 配置。
//!
//! 解析与合并全交给 `serde`：嵌套结构体直接对应嵌套 JSON，`#[serde(default)]`
//! 表示"缺键保留默认值"。
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
            Ok(text) => Self::parse(&path, &text)?,
            // 文件不存在就用默认值；其它读错误（权限等）要报出来
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Config::default(),
            Err(err) => {
                return Err(err).with_context(|| format!("无法读取配置文件 {}", path.display()))
            }
        };
        config.absolutize_root();
        Ok(config)
    }

    /// [`Config::load`] 的同步版本，只在**启动时**读一次配置。
    ///
    /// GUI 必须在 GLib 主循环启动前拿到配置，CLI 也走同一份；其余地方都用 [`Config::load`]。
    pub fn load_sync(path: Option<&Path>) -> Result<Self> {
        let path = path.map(Path::to_path_buf).unwrap_or_else(Self::path);
        let mut config = match std::fs::read_to_string(&path) {
            Ok(text) => Self::parse(&path, &text)?,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Config::default(),
            Err(err) => {
                return Err(err).with_context(|| format!("无法读取配置文件 {}", path.display()))
            }
        };
        config.absolutize_root();
        Ok(config)
    }

    /// 解析配置文本（`load` / `load_sync` 共用）。
    fn parse(path: &Path, text: &str) -> Result<Self> {
        serde_json::from_str(text).with_context(|| format!("无法解析配置文件 {}", path.display()))
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

        let loaded = Config::load_sync(Some(&path)).unwrap();
        assert_eq!(loaded.account.username, "tester");
        assert_eq!(loaded.launch.minecraft_root_path, absolute("mc-root"));

        // 文件不存在 ⇒ 默认值（而不是报错）
        assert_eq!(
            Config::load_sync(Some(&dir.join("missing.json")))
                .unwrap()
                .account
                .username,
            "player"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
