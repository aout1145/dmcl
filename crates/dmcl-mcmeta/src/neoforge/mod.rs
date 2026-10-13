//! NeoForge 的元数据、地址与取数（版本列表、安装器地址、安装器档案）。

use std::cmp::Ordering;

use anyhow::Result;
use serde::Deserialize;
use version_compare::{compare, Cmp};

use crate::source::{DownloadSource, Root};

pub mod install_profile;
pub use install_profile::{DataEntry, DataPair, InstallProfile, Processor, Side};

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

/// NeoForge 版本列表地址。
///
/// **占位**：借用 bmclapi 的 JSON 路径（`${MC_META}` 的 bmclapi 侧就是站点根），官方侧
/// 解析出来是 `piston-meta` 下的地址、会 404 —— 靠 `fetch_json` 的候选轮换重试落到
/// bmclapi。TODO：官方 neoforged maven 与 bmclapi 都有 `maven-metadata.xml`，以后两边
/// 统一改用 XML 取，这里换成对应的模板。
fn neoforge_version_list_url(mc_version: &str) -> String {
    format!("{}/neoforge/list/{mc_version}", Root::McMeta.placeholder())
}

impl DownloadSource {
    /// 取 NeoForge 版本列表。
    pub async fn fetch_neoforge_versions(&self, mc_version: &str) -> Result<Vec<String>> {
        let listing: Vec<NeoForgeListEntry> = self
            .fetch_json(&neoforge_version_list_url(mc_version))
            .await?;
        Ok(listing.into_iter().map(|entry| entry.version).collect())
    }
}

/// 最新 NeoForge —— 按版本号取最大者。
pub async fn latest_neoforge_version(source: &DownloadSource, mc_version: &str) -> Result<String> {
    let versions = source.fetch_neoforge_versions(mc_version).await?;
    Ok(versions
        .into_iter()
        .max_by(|a, b| match compare(a, b) {
            Ok(Cmp::Lt) => Ordering::Less,
            Ok(Cmp::Gt) => Ordering::Greater,
            _ => Ordering::Equal,
        })
        .unwrap_or_default())
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
    fn neoforge_version_list_url_is_a_placeholder() {
        assert_eq!(
            neoforge_version_list_url("1.21.1"),
            "${MC_META}/neoforge/list/1.21.1"
        );
        // bmclapi 侧解析出来就是它真正的 JSON 地址；官方侧会 404（靠重试兜过去）
        assert_eq!(
            crate::source::resolve_url(
                &neoforge_version_list_url("1.21.1"),
                crate::source::Mirror::Bmclapi
            ),
            "https://bmclapi2.bangbang93.com/neoforge/list/1.21.1"
        );
    }
}
