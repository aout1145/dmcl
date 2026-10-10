//! 模组加载器的版本列表（只读）。
//!
//! 这些文档都只用来"列出可选版本 / 找最新版"，不写回任何东西，因此不保留未建模字段。

use anyhow::Result;
use serde::Deserialize;

use crate::source::DownloadSource;

/// `bmclapi .../forge/minecraft/<mcver>` 的一项。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ForgeVersionEntry {
    #[serde(default)]
    pub mcversion: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub branch: Option<String>,
    /// build 号 —— 不可能为负。
    #[serde(default)]
    pub build: u64,
}

impl ForgeVersionEntry {
    /// `bmclapiGetForgeVersionName` —— `<mcversion>-<version>[-<branch>]`。
    pub fn full_name(&self) -> String {
        let mut name = format!("{}-{}", self.mcversion, self.version);
        if let Some(branch) = self.branch.as_deref() {
            // `("branch" in ver.object && !ver["branch"].isNull())`
            name.push('-');
            name.push_str(branch);
        }
        name
    }
}

/// 官方 neoforged maven 的版本列表接口。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct NeoForgeVersionListing {
    #[serde(default)]
    pub versions: Vec<String>,
}

/// `bmclapi .../neoforge/list/<mcver>` 的一项。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct NeoForgeListEntry {
    #[serde(default)]
    pub version: String,
}

/// `meta.fabricmc.net/v2/versions/loader/<mcver>` 的一项。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct FabricLoaderEntry {
    #[serde(default)]
    pub loader: FabricLoader,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct FabricLoader {
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub stable: Option<bool>,
}

impl FabricLoader {
    pub fn is_stable(&self) -> bool {
        self.stable.unwrap_or(false)
    }
}

// ---------------------------------------------------------------------------
// 查询 / 找最新
// ---------------------------------------------------------------------------

/// Fabric 的 loader 列表（远端顺序）。
pub async fn fabric_loaders(
    source: &DownloadSource,
    mc_version: &str,
) -> Result<Vec<FabricLoaderEntry>> {
    source.fetch_fabric_loaders(mc_version).await
}

/// 最新的**稳定** Fabric loader；没有稳定版时取列表第一个（与原实现一致）。
pub async fn latest_fabric_loader(source: &DownloadSource, mc_version: &str) -> Result<String> {
    let list = fabric_loaders(source, mc_version).await?;
    for entry in list.iter().rev() {
        if entry.loader.is_stable() {
            return Ok(entry.loader.version.clone());
        }
    }
    Ok(list
        .first()
        .map(|entry| entry.loader.version.clone())
        .unwrap_or_default())
}

/// Forge 版本列表（bmclapi only），按 build 升序。
pub async fn forge_versions(
    source: &DownloadSource,
    mc_version: &str,
) -> Result<Vec<ForgeVersionEntry>> {
    let mut versions = source.fetch_forge_versions(mc_version).await?;
    versions.sort_by_key(|entry| entry.build);
    Ok(versions)
}

/// 最新 Forge（build 号最大）。
pub async fn latest_forge_version(source: &DownloadSource, mc_version: &str) -> Result<String> {
    let versions = forge_versions(source, mc_version).await?;
    Ok(versions
        .iter()
        .max_by_key(|entry| entry.build)
        .map(ForgeVersionEntry::full_name)
        .unwrap_or_default())
}

/// NeoForge 版本列表。
pub async fn neoforge_versions(source: &DownloadSource, mc_version: &str) -> Result<Vec<String>> {
    source.fetch_neoforge_versions(mc_version).await
}

/// 最新 NeoForge —— 原实现取列表的**最后一个**元素。
pub async fn latest_neoforge_version(source: &DownloadSource, mc_version: &str) -> Result<String> {
    let versions = neoforge_versions(source, mc_version).await?;
    Ok(versions.last().cloned().unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forge_version_names_include_the_branch_when_present() {
        let entry: ForgeVersionEntry =
            serde_json::from_str(r#"{"mcversion":"1.7.10","version":"10.13.4.1614","build":1614}"#)
                .unwrap();
        assert_eq!(entry.full_name(), "1.7.10-10.13.4.1614");

        let entry: ForgeVersionEntry = serde_json::from_str(
            r#"{"mcversion":"1.20.1","version":"47.2.0","branch":"recommended","build":1637}"#,
        )
        .unwrap();
        assert_eq!(entry.full_name(), "1.20.1-47.2.0-recommended");

        // `"branch": null` 时不该多出一个横杠
        let entry: ForgeVersionEntry =
            serde_json::from_str(r#"{"mcversion":"1.20.1","version":"47.2.0","branch":null}"#)
                .unwrap();
        assert_eq!(entry.full_name(), "1.20.1-47.2.0");
    }

    #[test]
    fn fabric_loader_stability_defaults_to_false() {
        let list: Vec<FabricLoaderEntry> = serde_json::from_str(
            r#"[{"loader":{"version":"0.15.0","stable":true}},{"loader":{"version":"0.16.0"}}]"#,
        )
        .unwrap();
        assert!(list[0].loader.is_stable());
        assert!(!list[1].loader.is_stable());
    }
}
