//! NeoForge 的元数据、地址与取数（版本列表、安装器地址、安装器档案）。

use anyhow::Result;
use serde::Deserialize;

use crate::source::{DownloadSource, Mirror, Root};

pub mod install_profile;
pub use install_profile::{DataEntry, DataPair, InstallProfile, Processor, Side};

/// bmclapi 的 NeoForge 版本列表接口。
pub const NEOFORGE_VERSION_LIST_BMCLAPI: &str = "https://bmclapi2.bangbang93.com/neoforge/list";

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
    /// 带 `neoforge-` 前缀的原始版本名。
    #[serde(
        default,
        rename = "rawVersion",
        skip_serializing_if = "Option::is_none"
    )]
    pub raw_version: Option<String>,
    #[serde(default, rename = "mcversion", skip_serializing_if = "Option::is_none")]
    pub mc_version: Option<String>,
    /// 安装器在镜像上的路径。
    #[serde(
        default,
        rename = "installerPath",
        skip_serializing_if = "Option::is_none"
    )]
    pub installer_path: Option<String>,
    /// 未建模：bmclapi 的 Mongo 记账字段（`_id` / `__v`）。
    #[serde(flatten)]
    pub extra: crate::json::JsonMap,
}

/// NeoForge 安装器（1.20.1 的 NeoForge 实际发布在 `net.neoforged:forge` 下）。
pub fn neoforge_installer_url(forge_version: &str) -> (String, &'static str) {
    let name = if forge_version.starts_with("1.20.1") {
        "forge"
    } else {
        "neoforge"
    };
    (
        format!(
            "{}/net/neoforged/{name}/{forge_version}/{name}-{forge_version}-installer.jar",
            Root::NeoForgeMaven.placeholder()
        ),
        name,
    )
}

/// 官方 neoforged maven 的版本列表地址。
pub fn neoforge_version_list_url_official(mc_version: &str) -> String {
    let repository = if mc_version == "1.20.1" {
        "forge"
    } else {
        "neoforge"
    };
    format!("https://maven.neoforged.net/api/maven/versions/releases/net/neoforged/{repository}")
}

/// bmclapi 的 NeoForge 版本列表地址。
pub fn neoforge_version_list_url_bmclapi(mc_version: &str) -> String {
    format!("{NEOFORGE_VERSION_LIST_BMCLAPI}/{mc_version}")
}

/// 按镜像选择 NeoForge 版本列表地址 —— 对应原实现里 `list_for_mirror` 的穷尽 `match`。
pub fn neoforge_version_list_url(mc_version: &str, mirror: Mirror) -> String {
    match mirror {
        Mirror::Official => neoforge_version_list_url_official(mc_version),
        Mirror::Bmclapi => neoforge_version_list_url_bmclapi(mc_version),
    }
}

impl DownloadSource {
    /// 取 NeoForge 版本列表；官方源与 bmclapi 的接口形态不同，这里统一成字符串列表。
    pub async fn fetch_neoforge_versions(&self, mc_version: &str) -> Result<Vec<String>> {
        Ok(match self.mirror() {
            Mirror::Official => {
                let listing: NeoForgeVersionListing = self
                    .get_json(&neoforge_version_list_url_official(mc_version))
                    .await?;
                // 官方接口返回整个仓库的版本，需要按 mc 版本前缀过滤：
                // 1.20.1 时代的前缀就是 "1.20.1"，之后是去掉 "1." 的 mc 版本号
                let prefix = if mc_version == "1.20.1" {
                    "1.20.1".to_string()
                } else {
                    mc_version.get(2..).unwrap_or("").to_string()
                };
                listing
                    .versions
                    .into_iter()
                    .filter(|version| version.starts_with(&prefix))
                    .collect()
            }
            Mirror::Bmclapi => {
                let listing: Vec<NeoForgeListEntry> = self
                    .get_json(&neoforge_version_list_url_bmclapi(mc_version))
                    .await?;
                listing.into_iter().map(|entry| entry.version).collect()
            }
        })
    }
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
    fn neoforge_installer_uses_the_legacy_artifact_for_1_20_1() {
        let (url, name) = neoforge_installer_url("1.20.1-47.1.106");
        assert_eq!(name, "forge");
        assert!(url.contains("/net/neoforged/forge/1.20.1-47.1.106/forge-"));
        let (url, name) = neoforge_installer_url("21.1.72");
        assert_eq!(name, "neoforge");
        assert!(url.contains("/net/neoforged/neoforge/21.1.72/neoforge-"));
    }

    #[test]
    fn neoforge_list_entries_keep_mirror_metadata() {
        let entry: NeoForgeListEntry = serde_json::from_str(
            r#"{"version":"21.1.257","rawVersion":"neoforge-21.1.257","mcversion":"1.21.1",
                "installerPath":"/maven/net/neoforged/neoforge/21.1.257/neoforge-21.1.257-installer.jar"}"#,
        )
        .unwrap();
        assert_eq!(entry.version, "21.1.257");
        assert_eq!(entry.raw_version.as_deref(), Some("neoforge-21.1.257"));
        assert_eq!(entry.mc_version.as_deref(), Some("1.21.1"));
        assert!(entry
            .installer_path
            .as_deref()
            .unwrap()
            .ends_with("-installer.jar"));
    }

    #[test]
    fn neoforge_version_list_depends_on_the_mirror() {
        assert_eq!(
            neoforge_version_list_url("1.20.1", Mirror::Official),
            "https://maven.neoforged.net/api/maven/versions/releases/net/neoforged/forge"
        );
        assert_eq!(
            neoforge_version_list_url("1.21.1", Mirror::Official),
            "https://maven.neoforged.net/api/maven/versions/releases/net/neoforged/neoforge"
        );
        assert_eq!(
            neoforge_version_list_url("1.21.1", Mirror::Bmclapi),
            "https://bmclapi2.bangbang93.com/neoforge/list/1.21.1"
        );
    }
}
