//! `search-java` —— 扫描本机 Java，合并进全局配置并保存。

use std::path::Path;

use anyhow::Result;
use dmcl_launch::{default_java_roots, find_java};

use crate::config::{configs_from_details, Config, JavaConfigs};

/// 把扫描结果合并进 `javas`，返回按路径新增了几个。
///
/// 已有名字（手工条目、旧配置里的非哈希名字）全部保留；同一路径不会以不同名字
/// 重复加入。同名条目更新整项，同一路径的其它名字只刷新缓存的版本 / 架构。
pub(crate) fn merge_javas(javas: &mut JavaConfigs, found: &JavaConfigs) -> usize {
    let mut added = 0;
    for (name, java) in found {
        let existing_name = javas
            .iter()
            .find(|(_, existing)| same_java_path(&existing.path, &java.path))
            .map(|(existing_name, _)| existing_name.clone());
        if let Some(existing_name) = existing_name {
            if existing_name == *name {
                javas.insert(name.clone(), java.clone());
            } else if let Some(existing) = javas.get_mut(&existing_name) {
                existing.major = java.major;
                existing.arch = java.arch;
            }
            continue;
        }
        if javas.insert(name.clone(), java.clone()).is_none() {
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
    let found = configs_from_details(find_java(&default_java_roots()).await);
    let added = merge_javas(&mut config.javas, &found);
    config.save().await?;

    println!("搜索到 {} 个 Java，新加入 {added} 个。", found.len());
    if config.javas.is_empty() {
        println!("还没有配置任何 Java。");
    } else {
        println!("已配置的 Java：");
        for (name, java) in &config.javas {
            println!(
                "{name}: {}（Java {} / {}）",
                java.path.display(),
                java.major,
                java.arch.name()
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use dmcl_mcmeta::os::Arch;

    use super::*;
    use crate::config::JavaConfig;

    fn java(path: &str, major: u32) -> JavaConfig {
        JavaConfig {
            path: PathBuf::from(path),
            major,
            arch: Arch::X86_64,
        }
    }

    #[test]
    fn merge_keeps_manual_names_and_deduplicates_by_path() {
        let mut javas = JavaConfigs::from([
            ("manual".to_string(), java("/manual/java", 17)),
            ("same".to_string(), java("/old/java", 11)),
        ]);

        let found = JavaConfigs::from([
            ("jdk17-hash".to_string(), java("/jdk17/bin/java", 21)),
            ("manual-hash".to_string(), java("/manual/java", 21)),
            ("manual".to_string(), java("/manual/java", 21)),
            ("same".to_string(), java("/new/java", 17)),
        ]);

        assert_eq!(merge_javas(&mut javas, &found), 1);
        assert_eq!(javas.len(), 3);
        assert!(!javas.contains_key("manual-hash"));
        assert_eq!(javas.get("manual"), Some(&java("/manual/java", 21)));
        assert_eq!(javas.get("same"), Some(&java("/new/java", 17)));
        assert_eq!(javas.get("jdk17-hash"), Some(&java("/jdk17/bin/java", 21)));
    }
}
