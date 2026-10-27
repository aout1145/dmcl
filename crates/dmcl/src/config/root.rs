//! 具名的 Minecraft 根目录：默认值、解析、校验与取用。
//!
//! 配置里保存的是原始路径，可以是相对路径；取用根目录时才绝对化。因此读写配置不会把
//! 路径改写成绝对路径。

use std::path::PathBuf;

use anyhow::{anyhow, Result};
use dmcl_mcmeta::layout::absolute;
use indexmap::IndexMap;
use serde_json::Value;

use super::{Config, ConfigWarning};

/// 内置根目录「当前目录」的名字。
pub(super) const CURRENT_ROOT_NAME: &str = "当前目录";

/// 内置根目录「官方启动器」的名字。
pub(super) const OFFICIAL_ROOT_NAME: &str = "官方启动器";

/// 默认根目录表。
pub(super) fn default_roots() -> IndexMap<String, PathBuf> {
    IndexMap::from([
        (CURRENT_ROOT_NAME.to_string(), PathBuf::from(".minecraft")),
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
    /// 所有具名根目录：名字 → 绝对化后的路径，保持配置里的顺序。
    pub fn roots(&self) -> Vec<(String, PathBuf)> {
        self.minecraft_roots
            .iter()
            .map(|(name, path)| (name.clone(), absolute(path)))
            .collect()
    }

    /// 按名字取绝对化后的根目录路径。
    pub fn root_path(&self, name: &str) -> Result<PathBuf> {
        self.minecraft_roots
            .get(name)
            .map(absolute)
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

    /// 新增根目录；名字要非空且不能与已有项重名。
    pub fn add_minecraft_root(&mut self, name: &str, path: PathBuf) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            return Err(anyhow!("目录名称不能为空"));
        }
        if self.minecraft_roots.contains_key(name) {
            return Err(anyhow!("已存在同名目录：{name}"));
        }
        if path.as_os_str().is_empty() {
            return Err(anyhow!("目录路径不能为空"));
        }
        self.minecraft_roots.insert(name.to_string(), path);
        Ok(())
    }

    /// 修改根目录的显示名，保持表里的顺序；默认目录跟着改名。
    pub fn rename_minecraft_root(&mut self, from: &str, to: &str) -> Result<()> {
        let from = from.trim();
        let to = to.trim();
        if !self.minecraft_roots.contains_key(from) {
            return Err(anyhow!("根目录不存在：{from}"));
        }
        if from == to {
            return Ok(());
        }
        if to.is_empty() {
            return Err(anyhow!("目录名称不能为空"));
        }
        if self.minecraft_roots.contains_key(to) {
            return Err(anyhow!("已存在同名目录：{to}"));
        }

        let index = self
            .minecraft_roots
            .get_index_of(from)
            .expect("上面已经确认根目录存在");
        let path = self
            .minecraft_roots
            .shift_remove(from)
            .expect("上面已经确认根目录存在");
        self.minecraft_roots
            .shift_insert(index, to.to_string(), path);
        if self.minecraft_root == from {
            self.minecraft_root = to.to_string();
        }
        Ok(())
    }

    /// 修改根目录的路径；名字必须已经存在。
    pub fn set_minecraft_root_path(&mut self, name: &str, path: PathBuf) -> Result<()> {
        let Some(slot) = self.minecraft_roots.get_mut(name) else {
            return Err(anyhow!("根目录不存在：{name}"));
        };
        if path.as_os_str().is_empty() {
            return Err(anyhow!("目录路径不能为空"));
        }
        *slot = path;
        Ok(())
    }

    /// 把根目录移动到 `to`（移动后的下标）；默认目录不变。
    pub fn move_minecraft_root(&mut self, name: &str, to: usize) -> Result<()> {
        let from = self
            .minecraft_roots
            .get_index_of(name)
            .ok_or_else(|| anyhow!("根目录不存在：{name}"))?;
        if to >= self.minecraft_roots.len() {
            return Err(anyhow!("目标位置超出范围：{to}"));
        }
        if from == to {
            return Ok(());
        }

        let path = self
            .minecraft_roots
            .shift_remove(name)
            .expect("上面已经确认根目录存在");
        self.minecraft_roots
            .shift_insert(to, name.to_string(), path);
        Ok(())
    }

    /// 删除根目录；至少保留一个，删掉默认项时改用其余目录里的一个。
    pub fn remove_minecraft_root(&mut self, name: &str) -> Result<()> {
        if !self.minecraft_roots.contains_key(name) {
            return Err(anyhow!("根目录不存在：{name}"));
        }
        if self.minecraft_roots.len() <= 1 {
            return Err(anyhow!("至少保留一个 Minecraft 目录"));
        }
        self.minecraft_roots.shift_remove(name);

        if !self.minecraft_roots.contains_key(&self.minecraft_root) {
            self.minecraft_root = if self.minecraft_roots.contains_key(CURRENT_ROOT_NAME) {
                CURRENT_ROOT_NAME.to_string()
            } else {
                self.minecraft_roots
                    .keys()
                    .next()
                    .cloned()
                    .expect("刚删掉一项后至少还剩一项")
            };
        }
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
                    "默认根目录为 {CURRENT_ROOT_NAME} -> .minecraft、\
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
            Some(&PathBuf::from(".minecraft"))
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
    fn adding_a_root_rejects_empty_or_duplicate_names() {
        let mut config = Config::default();
        config
            .add_minecraft_root("test", PathBuf::from("/mc"))
            .unwrap();
        assert_eq!(
            config.minecraft_roots.get("test"),
            Some(&PathBuf::from("/mc"))
        );

        assert!(config
            .add_minecraft_root("  ", PathBuf::from("/other"))
            .is_err());
        assert!(config
            .add_minecraft_root("test", PathBuf::from("/other"))
            .is_err());
        assert_eq!(config.roots().len(), 3);
    }

    #[test]
    fn renaming_a_root_keeps_order_and_follows_the_default() {
        let mut config = Config::default();
        config
            .add_minecraft_root("test", PathBuf::from("/mc"))
            .unwrap();
        config.select_minecraft_root("test").unwrap();

        config.rename_minecraft_root("test", "renamed").unwrap();

        let names: Vec<String> = config.roots().into_iter().map(|(name, _)| name).collect();
        assert_eq!(
            names,
            vec![
                CURRENT_ROOT_NAME.to_string(),
                OFFICIAL_ROOT_NAME.to_string(),
                "renamed".to_string()
            ]
        );
        assert_eq!(config.minecraft_root, "renamed");
        assert_eq!(config.root_path("renamed").unwrap(), absolute("/mc"));

        // 改成已有名字或空名字都要拒绝，原表保持不变。
        assert!(config
            .rename_minecraft_root("renamed", CURRENT_ROOT_NAME)
            .is_err());
        assert!(config.rename_minecraft_root("renamed", "  ").is_err());
        assert_eq!(config.minecraft_root, "renamed");
        assert!(config.root_path("renamed").is_ok());
    }

    #[test]
    fn changing_a_root_path_requires_an_existing_name() {
        let mut config = Config::default();
        config
            .set_minecraft_root_path(CURRENT_ROOT_NAME, PathBuf::from("/new"))
            .unwrap();
        assert_eq!(
            config.root_path(CURRENT_ROOT_NAME).unwrap(),
            absolute("/new")
        );
        assert!(config
            .set_minecraft_root_path("nope", PathBuf::from("/new"))
            .is_err());
    }

    #[test]
    fn moving_a_root_reorders_without_changing_the_default() {
        let mut config = Config::default();
        config
            .add_minecraft_root("test", PathBuf::from("/mc"))
            .unwrap();

        config.move_minecraft_root("test", 0).unwrap();
        let names: Vec<String> = config.roots().into_iter().map(|(name, _)| name).collect();
        assert_eq!(
            names,
            vec![
                "test".to_string(),
                CURRENT_ROOT_NAME.to_string(),
                OFFICIAL_ROOT_NAME.to_string()
            ]
        );
        assert_eq!(config.minecraft_root, CURRENT_ROOT_NAME);

        config.move_minecraft_root("test", 2).unwrap();
        let names: Vec<String> = config.roots().into_iter().map(|(name, _)| name).collect();
        assert_eq!(
            names,
            vec![
                CURRENT_ROOT_NAME.to_string(),
                OFFICIAL_ROOT_NAME.to_string(),
                "test".to_string()
            ]
        );
        assert_eq!(config.root_path("test").unwrap(), absolute("/mc"));

        assert!(config.move_minecraft_root("nope", 0).is_err());
        assert!(config.move_minecraft_root("test", 3).is_err());
    }

    #[test]
    fn removing_a_root_keeps_at_least_one_and_repairs_the_default() {
        let mut config = Config::default();
        config
            .add_minecraft_root("test", PathBuf::from("/mc"))
            .unwrap();
        config.select_minecraft_root("test").unwrap();

        // 删掉默认项：回退到内置的「当前目录」。
        config.remove_minecraft_root("test").unwrap();
        assert!(!config.minecraft_roots.contains_key("test"));
        assert_eq!(config.minecraft_root, CURRENT_ROOT_NAME);

        config.remove_minecraft_root(OFFICIAL_ROOT_NAME).unwrap();
        let err = config.remove_minecraft_root(CURRENT_ROOT_NAME).unwrap_err();
        assert!(err.to_string().contains("至少保留一个"));
        assert!(config.root_path(CURRENT_ROOT_NAME).is_ok());
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
    fn relative_paths_are_saved_as_is_and_resolved_on_use() {
        let mut config = Config::default();
        config
            .minecraft_roots
            .insert("minecraft".to_string(), PathBuf::from("mc"));

        let text = serde_json::to_string_pretty(&config).unwrap();
        assert!(text.contains("\".minecraft\""), "{text}");
        assert!(text.contains("\"mc\""), "{text}");

        let (loaded, warnings) = Config::parse(&text);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(loaded.default_root_path().unwrap(), absolute(".minecraft"));
        assert_eq!(loaded.root_path("minecraft").unwrap(), absolute("mc"));
        assert_eq!(
            loaded.minecraft_roots.get("minecraft"),
            Some(&PathBuf::from("mc"))
        );
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
