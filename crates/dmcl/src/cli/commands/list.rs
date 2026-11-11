//! `list` / `list-*` —— 列出已安装的版本，或可安装的版本。

use std::path::Path;

use anyhow::Result;
use dialoguer::console::Style;
use dmcl_game::{detect_version, DetectedVersion, VersionRef};

use crate::config::Config;

/// `list` —— 列出**已安装**的版本。
pub(crate) async fn installed(root_path: &Path) -> Result<()> {
    let versions = dmcl_mcmeta::layout::installed_versions(root_path).await?;
    if versions.is_empty() {
        println!("还没有安装任何版本。");
        return Ok(());
    }
    println!("已安装的版本：");
    for version in versions {
        let detected = detect_version(&VersionRef::new(root_path, &version.name)).await;
        match detected.ok().and_then(|detected| summary(&detected)) {
            Some(summary) => println!("{} {}", version.name, Style::new().dim().apply_to(summary)),
            None => println!("{}", version.name),
        }
    }
    Ok(())
}

/// 把识别结果压成名称后面的一段说明；没有任何信息时返回 `None`。
fn summary(detected: &DetectedVersion) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(minecraft) = &detected.minecraft {
        parts.push(minecraft.clone());
    }
    for loader in &detected.loaders {
        let name = loader.kind.display_name();
        parts.push(match &loader.version {
            Some(version) => format!("{name} {version}"),
            None => name.to_string(),
        });
    }
    (!parts.is_empty()).then(|| parts.join(", "))
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

/// `list-optifine`。
pub(crate) async fn optifine(config: &Config, verid: &str) -> Result<()> {
    let source = config.download_source();
    // fetch 已按稳定版 / 预览版与版本号从新到旧排序
    for entry in source.fetch_optifine_versions(verid).await? {
        println!(
            "{}{}",
            entry.version(),
            if entry.is_preview() {
                " (预览版)"
            } else {
                ""
            }
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dmcl_game::{DetectedLoader, DetectedLoaderKind};

    fn loader(kind: DetectedLoaderKind, version: Option<&str>) -> DetectedLoader {
        DetectedLoader {
            kind,
            version: version.map(str::to_owned),
        }
    }

    #[test]
    fn summary_puts_the_minecraft_version_before_the_loaders() {
        let detected = DetectedVersion {
            minecraft: Some("1.12.2".to_string()),
            loaders: vec![loader(DetectedLoaderKind::Forge, Some("x.xx"))],
        };

        assert_eq!(summary(&detected).as_deref(), Some("1.12.2, Forge x.xx"));
    }

    #[test]
    fn summary_skips_unknown_parts() {
        let detected = DetectedVersion {
            minecraft: None,
            loaders: vec![
                loader(DetectedLoaderKind::Fabric, Some("0.15.0")),
                loader(DetectedLoaderKind::NeoForge, None),
            ],
        };

        assert_eq!(
            summary(&detected).as_deref(),
            Some("Fabric 0.15.0, NeoForge")
        );
        assert_eq!(summary(&DetectedVersion::default()), None);
    }
}
