//! Forge 的元数据、地址与取数（版本列表、安装器地址、安装器档案）。

use anyhow::Result;
use serde::Deserialize;

use crate::source::{DownloadSource, Root};

pub mod install_profile;
pub use install_profile::{DataEntry, DataPair, InstallEntry, InstallProfile, Processor, Side};

/// Forge 的版本列表接口 —— 目前只有 bmclapi 提供。
///
/// TODO：官方 maven 也有 `maven-metadata.xml`（XML，形如 `1.21.1-52.1.16`，
/// 但没有 `branch` / `build`），NeoForge 的官方 maven 同理。以后把两边官方源的
/// 列表统一改用 XML 取，再决定要不要保留这里的 bmclapi 接口。
pub const FORGE_VERSION_LIST: &str = "https://bmclapi2.bangbang93.com/forge/minecraft";

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
    /// 该 build 下各类文件的下载信息。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub files: Option<Vec<ForgeFile>>,
    /// 最后修改时间（ISO 8601）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified: Option<String>,
    /// 未建模：bmclapi 的 Mongo 记账字段（`_id` / `__v`）。
    #[serde(flatten)]
    pub extra: crate::json::JsonMap,
}

/// Forge build 下的一个文件（changelog / installer / mdk / universal）。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ForgeFile {
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    /// 文件的 sha1。
    #[serde(default)]
    pub hash: Option<String>,
    #[serde(flatten)]
    pub extra: crate::json::JsonMap,
}

impl ForgeVersionEntry {
    /// `<mcversion>-<version>[-<branch>]`。
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

/// Forge 安装器：`${FORGE_MAVEN}/net/minecraftforge/forge/<ver>/forge-<ver>-installer.jar`。
pub fn forge_installer_url(forge_version: &str) -> String {
    format!(
        "{}/net/minecraftforge/forge/{forge_version}/forge-{forge_version}-installer.jar",
        Root::ForgeMaven.placeholder()
    )
}

/// Forge 版本列表地址（bmclapi only）。
pub fn forge_version_list_url(mc_version: &str) -> String {
    format!("{FORGE_VERSION_LIST}/{mc_version}")
}

impl DownloadSource {
    /// 取 Forge 版本列表（只有 bmclapi 提供）。
    pub async fn fetch_forge_versions(&self, mc_version: &str) -> Result<Vec<ForgeVersionEntry>> {
        self.get_json(&forge_version_list_url(mc_version)).await
    }
}

/// 最新 Forge（build 号最大）。
pub async fn latest_forge_version(source: &DownloadSource, mc_version: &str) -> Result<String> {
    let versions = source.fetch_forge_versions(mc_version).await?;
    Ok(versions
        .iter()
        .max_by_key(|entry| entry.build)
        .map(ForgeVersionEntry::full_name)
        .unwrap_or_default())
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
    fn forge_urls_are_built_as_templates() {
        assert_eq!(
            forge_installer_url("1.20.1-47.2.0"),
            "${FORGE_MAVEN}/net/minecraftforge/forge/1.20.1-47.2.0/forge-1.20.1-47.2.0-installer.jar"
        );
    }

    #[test]
    fn forge_list_entries_keep_files_and_modified() {
        let entry: ForgeVersionEntry = serde_json::from_str(
            r#"{"mcversion":"1.21.1","version":"52.1.16","build":52010016,
                "modified":"2026-07-21T11:11:51.000Z",
                "files":[{"format":"jar","category":"installer","hash":"abc"}],
                "_id":"x","__v":0}"#,
        )
        .unwrap();
        assert_eq!(
            entry.files.as_ref().unwrap()[0].category.as_deref(),
            Some("installer")
        );
        assert_eq!(entry.modified.as_deref(), Some("2026-07-21T11:11:51.000Z"));
        // 未建模的 Mongo 记账字段落在 extra
        assert!(entry.extra.contains_key("_id"));
    }

    #[test]
    fn the_forge_version_list_is_bmclapi_only() {
        assert_eq!(
            forge_version_list_url("1.20.1"),
            "https://bmclapi2.bangbang93.com/forge/minecraft/1.20.1"
        );
    }
}
