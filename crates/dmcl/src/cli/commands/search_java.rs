//! `search-java` —— 扫描本机 Java，合并进全局配置并保存。

use anyhow::Result;
use dmcl_launch::{default_java_roots, find_java};

use crate::config::{configs_from_details, merge_javas, Config};

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
