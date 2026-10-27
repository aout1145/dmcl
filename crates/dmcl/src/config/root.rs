//! 具名的 Minecraft 根目录：默认值、解析、校验与取用。
//!
//! 配置里保存的是原始路径（可含 [`CWD_PLACEHOLDER`] 或相对路径）；取用根目录时展开
//! 占位符并绝对化，因此保存配置不会把当前目录写死。

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use dmcl_mcmeta::layout::absolute;
use indexmap::IndexMap;
use serde_json::Value;

use super::{Config, ConfigWarning};

/// 根目录路径里表示「当前工作目录」的占位符。
///
/// 它在取用根目录时展开，配置里始终保留原文，因此保存配置不会把当前目录写死。
pub const CWD_PLACEHOLDER: &str = "${CWD}";

/// 内置根目录「当前目录」的名字。
pub(super) const CURRENT_ROOT_NAME: &str = "当前目录";

/// 内置根目录「官方启动器」的名字。
pub(super) const OFFICIAL_ROOT_NAME: &str = "官方启动器";

/// 默认根目录表。
pub(super) fn default_roots() -> IndexMap<String, PathBuf> {
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

/// 解析根目录对象：名字 → 路径，条目逐个校验，坏条目丢弃。
pub(super) fn parse_roots(
    body: Value,
    warnings: &mut Vec<ConfigWarning>,
) -> IndexMap<String, PathBuf> {
    let Value::Object(entries) = body else {
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

/// 解析 `minecraft_root`：必须是字符串，否则保留原值并警告。
pub(super) fn parse_root_name(
    body: Value,
    fallback: Value,
    warnings: &mut Vec<ConfigWarning>,
) -> Value {
    if body.is_string() {
        body
    } else {
        warnings.push(ConfigWarning {
            message: format!("配置项「minecraft_root」使用默认值 {fallback}"),
            detail: "值不合法：需要字符串".to_string(),
        });
        fallback
    }
}

impl Config {
    /// 所有具名根目录：名字 → 已展开的路径，保持配置里的顺序。
    pub fn roots(&self) -> Vec<(String, PathBuf)> {
        self.minecraft_roots
            .iter()
            .map(|(name, path)| (name.clone(), resolve(path)))
            .collect()
    }

    /// 按名字取已展开的根目录路径。
    pub fn root_path(&self, name: &str) -> Result<PathBuf> {
        self.minecraft_roots
            .get(name)
            .map(|path| resolve(path))
            .ok_or_else(|| anyhow!("根目录不存在：{name}"))
    }

    /// `minecraft_root` 指向的默认根目录名字。
    pub fn default_root_name(&self) -> Result<&str> {
        let name = self.minecraft_root.as_str();
        self.minecraft_roots
            .contains_key(name)
            .then_some(name)
            .ok_or_else(|| {
                anyhow!(
                    "配置项「minecraft_root」指向的根目录不存在：{}",
                    self.minecraft_root
                )
            })
    }

    /// `minecraft_root` 指向的默认根目录路径。
    pub fn default_root_path(&self) -> Result<PathBuf> {
        self.root_path(self.default_root_name()?)
    }

    /// 修改默认根目录；名字必须已经在 `minecraft_roots` 里。
    pub fn select_minecraft_root(&mut self, name: &str) -> Result<()> {
        if !self.minecraft_roots.contains_key(name) {
            return Err(anyhow!("根目录不存在：{name}"));
        }
        self.minecraft_root = name.to_string();
        Ok(())
    }

    /// 校验根目录选择：空表补默认，失效的默认根目录回退并警告。
    pub(super) fn validate_roots(&mut self) -> Vec<ConfigWarning> {
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

        warnings
    }
}

/// 展开 `${CWD}` 并绝对化。
fn resolve(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if !text.contains(CWD_PLACEHOLDER) {
        return absolute(path);
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    absolute(text.replace(CWD_PLACEHOLDER, &cwd.to_string_lossy()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn default_roots_are_current_and_official() {
        let config = Config::default();
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
    fn bad_root_entries_are_dropped() {
        let (config, warnings) = Config::parse(
            r#"{
                "minecraft_roots": { "good": "/mc", "bad": 1 },
                "minecraft_root": "good"
            }"#,
        );
        assert!(config.root_path("good").is_ok());
        assert!(config.root_path("bad").is_err());
        assert!(!config.minecraft_roots.contains_key(CURRENT_ROOT_NAME));
        assert!(warnings
            .iter()
            .any(|warning| warning.message.contains("minecraft_roots.bad")));
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
}
