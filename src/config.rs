//! Configuration (was `dmcl.config`).
//!
//! The original implementation used D's compile-time reflection
//! (`__traits(allMembers, Config)` + `replaceFirst("_", ".")`) to map struct
//! fields onto dotted JSON keys, and hand-rolled the parsing/merging.
//!
//! Here that whole mechanism is replaced by `serde`: nested structs map
//! directly onto the nested JSON document (which is exactly what the dotted
//! keys produced), and `#[serde(default)]` reproduces the "missing key keeps
//! the default" behaviour of the original reader. No reflection, no manual
//! JSON walking.
//!
//! The resulting `dmcl.json` layout is byte-compatible with the D version:
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
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};

use anyhow::{Context, Result};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::download::mirrors::Mirror;
use crate::env::LAUNCHER_NAME;
use crate::utils::get_path;

/// The literal the D implementation used for "pick a java automatically".
pub const JAVA_AUTOSELECT: &str = "autoselect";

/// `launch.java` — either automatic selection or one key of `java_configs`.
///
/// The D code stored this as a bare string and compared it against
/// `"autoselect"` in several places. As an enum the sentinel cannot be
/// mistyped, while the serialised form is still the plain string the original
/// config used, so existing `dmcl.json` files keep working.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum JavaChoice {
    /// Written as `"autoselect"`.
    #[default]
    Auto,
    /// The name of an entry in [`LaunchConfig::java_configs`].
    Named(String),
}

impl JavaChoice {
    pub fn as_str(&self) -> &str {
        match self {
            JavaChoice::Auto => JAVA_AUTOSELECT,
            JavaChoice::Named(name) => name,
        }
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

/// `launch.*` section.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LaunchConfig {
    /// Named java installations (`name -> executable`), filled by `search-java`.
    pub java_configs: BTreeMap<String, PathBuf>,
    /// Which of `java_configs` to use, or automatic selection.
    pub java: JavaChoice,
    pub minecraft_root_path: PathBuf,
    /// Window size in pixels — never negative.
    pub window_width: u32,
    pub window_height: u32,
    /// JVM heap bounds in MiB — never negative.
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
            additional_jvm:
                "-Dfml.ignoreInvalidMinecraftCertificates=True -Dfml.ignorePatchDiscrepancies=True"
                    .to_string(),
            additional_game: String::new(),
        }
    }
}

/// `account.*` section.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AccountConfig {
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

/// `download.*` section.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DownloadConfig {
    /// Maximum number of concurrent downloads (semaphore permits).
    pub max_tasks: usize,
    /// Skip downloading `*.ogg` assets (music & sounds).
    pub i_dont_need_music: bool,
    pub mirror: Mirror,
}

impl Default for DownloadConfig {
    fn default() -> Self {
        Self {
            max_tasks: 64,
            i_dont_need_music: false,
            mirror: Mirror::default(),
        }
    }
}

/// `cli.*` section.
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

/// The whole configuration document.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub launch: LaunchConfig,
    pub account: AccountConfig,
    pub download: DownloadConfig,
    pub cli: CliConfig,
}

/// Global configuration, mirroring the original global `config` variable.
/// A `RwLock` keeps it sound under the multi-threaded async runtime, and lets a
/// future GUI replace the whole document atomically via [`set_config`].
static CONFIG: OnceLock<RwLock<Config>> = OnceLock::new();

fn store() -> &'static RwLock<Config> {
    CONFIG.get_or_init(|| RwLock::new(Config::default()))
}

/// Read access to the global configuration.
pub fn with_config<R>(f: impl FnOnce(&Config) -> R) -> R {
    let guard = store().read().unwrap_or_else(|e| e.into_inner());
    f(&guard)
}

/// Write access to the global configuration.
pub fn with_config_mut<R>(f: impl FnOnce(&mut Config) -> R) -> R {
    let mut guard = store().write().unwrap_or_else(|e| e.into_inner());
    f(&mut guard)
}

/// Replace the whole configuration (used by the GUI / tests).
pub fn set_config(config: Config) {
    *store().write().unwrap_or_else(|e| e.into_inner()) = config;
}

/// `getConfigPath` — `<platform config dir>/dmcl.json`.
///
/// The base directory comes from [`dirs`] instead of hand-picking
/// `%APPDATA%` / `$HOME/.config`:
///
/// * Linux — `$XDG_CONFIG_HOME` or `~/.config` → same file as before
/// * Windows — `%APPDATA%` → same file as before
/// * macOS — `~/Library/Application Support` → *was* `~/.config`; the only
///   platform where the file moves (see MIGRATION.md)
pub fn get_config_path() -> PathBuf {
    match dirs::config_dir() {
        Some(base) => get_path(base.join("dmcl.json")),
        None => get_path("dmcl.json"),
    }
}

/// `readConfig` — load the config document (absent file ⇒ defaults) and make
/// the minecraft root path absolute.
pub async fn read_config(path: Option<&Path>) -> Result<()> {
    let path = path.map(Path::to_path_buf).unwrap_or_else(get_config_path);
    let mut config = match tokio::fs::read_to_string(&path).await {
        Ok(text) => serde_json::from_str(&text)
            .with_context(|| format!("failed to parse config {}", path.display()))?,
        Err(_) => Config::default(),
    };
    config.launch.minecraft_root_path = get_path(&config.launch.minecraft_root_path);
    set_config(config);
    Ok(())
}

/// `saveConfig` — persist the configuration. The D version merged only the
/// named members into the existing file; with serde the whole document is
/// serialised, which is equivalent because *all* members are always loaded.
pub async fn save_config() -> Result<()> {
    let path = get_config_path();
    let text = with_config(|c| serde_json::to_string_pretty(c).unwrap_or_default());
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.ok();
    }
    tokio::fs::write(&path, text)
        .await
        .with_context(|| format!("failed to write config {}", path.display()))?;
    Ok(())
}

/// `showConfig` — human readable dump (same field naming as D).
pub fn show_config() {
    println!("CONFIG PATH: {}\n", get_config_path().display());
    with_config(|c| {
        println!("launch.java = {}", c.launch.java);
        println!("launch.java_configs = {{{}}}", {
            c.launch
                .java_configs
                .iter()
                .map(|(name, path)| format!("{name}: {}", path.display()))
                .collect::<Vec<_>>()
                .join(", ")
        });
        println!(
            "launch.minecraft_root_path = {}",
            c.launch.minecraft_root_path.display()
        );
        println!("launch.window_width = {}", c.launch.window_width);
        println!("launch.window_height = {}", c.launch.window_height);
        println!("launch.min_memory = {}", c.launch.min_memory);
        println!("launch.max_memory = {}", c.launch.max_memory);
        println!("launch.custom_info = {}", c.launch.custom_info);
        println!("launch.additional_jvm = {}", c.launch.additional_jvm);
        println!("launch.additional_game = {}", c.launch.additional_game);
        println!("account.online_mode = {}", c.account.online_mode);
        println!("account.username = {}", c.account.username);
        println!("download.max_tasks = {}", c.download.max_tasks);
        println!(
            "download.i_dont_need_music = {}",
            c.download.i_dont_need_music
        );
        println!("download.mirror = {}", c.download.mirror);
        println!("cli.default_command = {}", c.cli.default_command);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_same_dotted_json_as_the_d_version() {
        // `launch_java` in D ⇒ `launch.java`, `download_mirror` ⇒ `download.mirror`, ...
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
        // absent keys keep the defaults, exactly like the original reader
        assert_eq!(config.launch.window_height, 480);
        assert_eq!(config.launch.min_memory, 512);
        assert_eq!(config.download.max_tasks, 64);
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
        assert!(value["cli"]["default_command"].is_string());
    }

    #[test]
    fn empty_document_yields_all_defaults() {
        let config: Config = serde_json::from_str("{}").unwrap();
        assert_eq!(config.launch.java, JavaChoice::Auto);
        assert_eq!(config.launch.custom_info, crate::env::LAUNCHER_NAME);
        assert_eq!(config.download.mirror, Mirror::Official);
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
    }

    #[test]
    fn negative_sizes_are_rejected_instead_of_wrapping() {
        // the D `int` accepted these and would have passed them to the JVM
        assert!(serde_json::from_str::<Config>(r#"{"launch":{"window_width":-1}}"#).is_err());
        assert!(serde_json::from_str::<Config>(r#"{"launch":{"max_memory":-2048}}"#).is_err());
    }
}
