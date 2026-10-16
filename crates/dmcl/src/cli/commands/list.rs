//! `list` / `list-*` —— 列出已安装的版本，或可安装的版本。

use anyhow::Result;

use crate::config::Config;

/// `list` —— 列出**已安装**的版本。
pub(crate) async fn installed(config: &Config) -> Result<()> {
    let versions =
        dmcl_mcmeta::layout::installed_versions(&config.launch.minecraft_root_path).await?;
    if versions.is_empty() {
        println!("还没有安装任何版本。");
        return Ok(());
    }
    println!("已安装的版本：");
    for version in versions {
        println!("{}", version.name);
    }
    Ok(())
}

/// `list-mc`。
pub(crate) async fn mc(config: &Config, kinds: &[String]) -> Result<()> {
    let source = config.download_source();
    let manifest = source.fetch_version_manifest().await?;
    // 从新到旧输出
    for version in manifest.filter_kinds(kinds).rev() {
        println!("{} {}", version.kind, version.id);
    }
    Ok(())
}

/// `list-forge`。
pub(crate) async fn forge(config: &Config, verid: &str) -> Result<()> {
    let source = config.download_source();
    // fetch 已按 build 从新到旧排序
    for version in source.fetch_forge_versions(verid).await? {
        println!("{}", version.full_name());
    }
    Ok(())
}

/// `list-neoforge`。
pub(crate) async fn neoforge(config: &Config, verid: &str) -> Result<()> {
    let source = config.download_source();
    // fetch 已按版本号从新到旧排序
    for entry in source.fetch_neoforge_versions(verid).await? {
        println!("{}", entry.version);
    }
    Ok(())
}

/// `list-fabric`。
pub(crate) async fn fabric(config: &Config, verid: &str) -> Result<()> {
    let source = config.download_source();
    // fetch 已按 loader 版本从新到旧排序
    for entry in source.fetch_fabric_loaders(verid).await? {
        println!(
            "{verid}/{}{}",
            entry.loader.version,
            if entry.loader.is_stable() {
                "(recommanded)"
            } else {
                ""
            }
        );
    }
    Ok(())
}
