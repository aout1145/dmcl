//! `search-java` —— 扫描本机 Java，合并进全局配置并保存。

use anyhow::Result;
use dmcl_launch::{find_java, JavaConfigs, DEFAULT_JAVA_ROOTS};

use crate::config::Config;

/// 把扫描结果合并进 `javas`，返回新增了几个。
///
/// 已存在的手工条目保留；同名条目更新路径。
pub(crate) fn merge_javas(javas: &mut JavaConfigs, found: &JavaConfigs) -> usize {
    let mut added = 0;
    for (name, path) in found {
        if javas.insert(name.clone(), path.clone()).is_none() {
            added += 1;
        }
    }
    added
}

/// `dmcl search-java`。
pub(crate) async fn run(config: &mut Config) -> Result<()> {
    let found = find_java(DEFAULT_JAVA_ROOTS).await?;
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
    fn merge_keeps_manual_entries_and_counts_new_names() {
        let mut javas = JavaConfigs::new();
        javas.insert("manual".to_string(), PathBuf::from("/manual/java"));
        javas.insert("same".to_string(), PathBuf::from("/old/java"));

        let mut found = JavaConfigs::new();
        found.insert("manual".to_string(), PathBuf::from("/manual/java"));
        found.insert("same".to_string(), PathBuf::from("/new/java"));
        found.insert("jdk17".to_string(), PathBuf::from("/jdk17/bin/java"));

        assert_eq!(merge_javas(&mut javas, &found), 1);
        assert_eq!(javas.get("manual"), Some(&PathBuf::from("/manual/java")));
        assert_eq!(javas.get("same"), Some(&PathBuf::from("/new/java")));
        assert_eq!(javas.get("jdk17"), Some(&PathBuf::from("/jdk17/bin/java")));
    }
}
