//! `search-java` —— 扫描本机 Java，合并进全局配置并保存。

use std::path::Path;

use anyhow::Result;
use dmcl_launch::{default_java_roots, find_java, JavaConfigs};

use crate::config::Config;

/// 把扫描结果合并进 `javas`，返回按路径新增了几个。
///
/// 已有名字（手工条目、旧配置里的非哈希名字）全部保留；同一路径不会以不同名字
/// 重复加入。同名条目仍然更新路径。
pub(crate) fn merge_javas(javas: &mut JavaConfigs, found: &JavaConfigs) -> usize {
    let mut added = 0;
    for (name, path) in found {
        let existing_name = javas
            .iter()
            .find(|(_, existing)| same_java_path(existing, path))
            .map(|(existing_name, _)| existing_name.clone());
        if let Some(existing_name) = existing_name {
            if existing_name == *name {
                javas.insert(name.clone(), path.clone());
            }
            continue;
        }
        if javas.insert(name.clone(), path.clone()).is_none() {
            added += 1;
        }
    }
    added
}

fn same_java_path(left: &Path, right: &Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

/// `dmcl search-java`。
pub(crate) async fn run(config: &mut Config) -> Result<()> {
    let found = find_java(&default_java_roots()).await;
    let added = merge_javas(&mut config.javas, &found);
    config.save().await?;

    println!("搜索到 {} 个 Java，新加入 {added} 个。", found.len());
    if config.javas.is_empty() {
        println!("还没有配置任何 Java。");
    } else {
        println!("已配置的 Java：");
        for (name, path) in &config.javas {
            println!("{name}: {}", path.display());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn merge_keeps_manual_names_and_deduplicates_by_path() {
        let mut javas = JavaConfigs::new();
        javas.insert("manual".to_string(), PathBuf::from("/manual/java"));
        javas.insert("same".to_string(), PathBuf::from("/old/java"));

        let mut found = JavaConfigs::new();
        found.insert("jdk17-hash".to_string(), PathBuf::from("/jdk17/bin/java"));
        found.insert("manual-hash".to_string(), PathBuf::from("/manual/java"));
        found.insert("manual".to_string(), PathBuf::from("/manual/java"));
        found.insert("same".to_string(), PathBuf::from("/new/java"));

        assert_eq!(merge_javas(&mut javas, &found), 1);
        assert_eq!(javas.len(), 3);
        assert!(!javas.contains_key("manual-hash"));
        assert_eq!(javas.get("manual"), Some(&PathBuf::from("/manual/java")));
        assert_eq!(javas.get("same"), Some(&PathBuf::from("/new/java")));
        assert_eq!(
            javas.get("jdk17-hash"),
            Some(&PathBuf::from("/jdk17/bin/java"))
        );
    }
}
