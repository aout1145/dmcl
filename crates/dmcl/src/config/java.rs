//! java 管理的纯逻辑（只改 [`Config`]，不跟终端打交道）。
//!
//! 从 `cli::commands::java` 挪出来，让 CLI 与 GUI 共用同一份：搜索合并、添加、
//! 选择、删除，以及"删掉正被选中的那个就回退到 autoselect"这类规则都在这儿。

use anyhow::{anyhow, Result};
use dmcl_launch::{JavaChoice, JavaConfigs, JavaOption};

use super::Config;

/// 把探测到的 java 合并进配置，返回新增了几个。
pub(crate) fn merge_java_configs(config: &mut Config, found: &JavaConfigs) -> usize {
    let mut added = 0;
    for (name, path) in found {
        if !config.launch.java_configs.contains_key(name) {
            added += 1;
        }
        config
            .launch
            .java_configs
            .insert(name.clone(), path.clone());
    }
    added
}

/// 把探测好的 java 写进配置，返回它占用的配置项名字。
///
/// 给了 `name` 就用它，否则用探测结果自动命名（`{type}-{os}-{arch}-{version}`）。
pub(crate) fn insert_java(config: &mut Config, java: &JavaOption, name: Option<&str>) -> String {
    let name = match name {
        Some(name) if !name.trim().is_empty() => name.trim().to_string(),
        _ => java.config_name(),
    };
    config
        .launch
        .java_configs
        .insert(name.clone(), java.path.clone());
    name
}

/// 设置当前选择；`autoselect` 表示自动，其余必须已经在配置里。
pub(crate) fn set_java_choice(config: &mut Config, name: &str) -> Result<()> {
    if name == "autoselect" {
        config.launch.java = JavaChoice::Auto;
        return Ok(());
    }
    if !config.launch.java_configs.contains_key(name) {
        return Err(anyhow!("没有这个 java 配置：{name}"));
    }
    config.launch.java = JavaChoice::Named(name.to_string());
    Ok(())
}

/// 删掉一个 java 配置；若它正被选中就回退为 `autoselect`。
///
/// 返回是否发生了回退。
pub(crate) fn remove_java(config: &mut Config, name: &str) -> Result<bool> {
    if config.launch.java_configs.remove(name).is_none() {
        return Err(anyhow!("没有这个 java 配置：{name}"));
    }
    let was_selected = config.launch.java == JavaChoice::Named(name.to_string());
    if was_selected {
        config.launch.java = JavaChoice::Auto;
    }
    Ok(was_selected)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn java(name: &str) -> JavaOption {
        JavaOption {
            path: PathBuf::from(format!("/opt/{name}/bin/java")),
            type_: "openjdk".to_string(),
            os: dmcl_mcmeta::os::Os::current(),
            arch: dmcl_mcmeta::os::Arch::X86_64,
            major_version: 17,
            version_str: "17.0.2".to_string(),
        }
    }

    #[test]
    fn search_merges_without_dropping_manual_entries() {
        let mut config = Config::default();
        config
            .launch
            .java_configs
            .insert("manual".to_string(), PathBuf::from("/manual/java"));
        let mut found = JavaConfigs::new();
        found.insert("jdk17".to_string(), PathBuf::from("/jdk17/bin/java"));
        found.insert("manual".to_string(), PathBuf::from("/manual/java"));

        let added = merge_java_configs(&mut config, &found);
        assert_eq!(added, 1);
        assert!(config.launch.java_configs.contains_key("manual"));
        assert!(config.launch.java_configs.contains_key("jdk17"));
    }

    #[test]
    fn add_uses_the_given_name_or_the_probed_one() {
        let mut config = Config::default();
        let jdk = java("jdk17");
        assert_eq!(insert_java(&mut config, &jdk, Some("myjdk")), "myjdk");
        // 留空 ⇒ 用探测结果命名
        assert_eq!(insert_java(&mut config, &jdk, None), jdk.config_name());
        assert!(config.launch.java_configs.contains_key("myjdk"));
        assert!(config.launch.java_configs.contains_key(&jdk.config_name()));
    }

    #[test]
    fn removing_the_selected_java_falls_back_to_autoselect() {
        let mut config = Config::default();
        config
            .launch
            .java_configs
            .insert("jdk".to_string(), PathBuf::from("/jdk/java"));
        config.launch.java = JavaChoice::Named("jdk".to_string());

        assert!(remove_java(&mut config, "jdk").unwrap());
        assert!(config.launch.java.is_auto());
        assert!(config.launch.java_configs.is_empty());

        // 删不存在的项要报错
        assert!(remove_java(&mut config, "jdk").is_err());
    }

    #[test]
    fn use_requires_an_existing_entry() {
        let mut config = Config::default();
        assert!(set_java_choice(&mut config, "nope").is_err());
        set_java_choice(&mut config, "autoselect").unwrap();
        assert!(config.launch.java.is_auto());

        config
            .launch
            .java_configs
            .insert("jdk".to_string(), PathBuf::from("/jdk/java"));
        set_java_choice(&mut config, "jdk").unwrap();
        assert_eq!(config.launch.java, JavaChoice::Named("jdk".to_string()));
    }
}
