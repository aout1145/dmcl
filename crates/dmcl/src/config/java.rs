//! `javas.*` 段与 `launch.java` 选择。

use std::path::PathBuf;

use anyhow::Result;
use dmcl_install::JavaProvider;
use dmcl_launch::{select_java, JavaChoice, JavaConfigs};
use serde_json::Value;

use super::{ConfigWarning, LaunchConfig};

/// 解析 `javas` 表：条目逐个校验，坏条目丢弃；表整体替换默认表。
pub(super) fn parse_javas(body: Value, warnings: &mut Vec<ConfigWarning>) -> JavaConfigs {
    super::parse_table("javas", body, JavaConfigs::new(), warnings)
}

/// `launch.java` 指向不存在的 Java 时回退为自动选择。
pub(super) fn validate_selection(
    launch: &mut LaunchConfig,
    javas: &JavaConfigs,
) -> Vec<ConfigWarning> {
    let mut warnings = Vec::new();

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

/// 让 `dmcl-install` 用配置里的 java 选择去跑安装器 processor。
pub(super) struct ConfigJava {
    pub(super) choice: JavaChoice,
    pub(super) configs: JavaConfigs,
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
    use crate::config::Config;

    #[test]
    fn javas_keep_the_file_order() {
        let text = r#"{
            "javas": { "b": "/b/java", "a": "/a/java" }
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
    }

    #[test]
    fn bad_java_entries_are_dropped() {
        let (config, warnings) =
            Config::parse(r#"{"javas": { "good": "/usr/bin/java", "bad": 1 }}"#);
        assert!(config.javas.contains_key("good"));
        assert!(!config.javas.contains_key("bad"));
        assert!(warnings
            .iter()
            .any(|warning| warning.message.contains("javas.bad")));
    }

    #[test]
    fn a_missing_named_java_falls_back_to_autoselect() {
        let (config, warnings) = Config::parse(r#"{"launch": {"java": "nope"}}"#);
        assert!(config.launch.java.is_auto());
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].message.contains("launch.java"));
    }
}
