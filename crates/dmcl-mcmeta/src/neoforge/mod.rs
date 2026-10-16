//! NeoForge 的元数据、地址与取数（版本列表、安装器地址、安装器档案）。

use std::cmp::Ordering;

use anyhow::Result;
use serde::Deserialize;
use version_compare::{compare, Cmp};

use crate::source::{DownloadSource, Mirror, Root};

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
/// **不可镜像**：这个 JSON 接口只有 bmclapi 提供，所以直接给出它的地址、不走镜像模板
/// （不含占位符，`fetch_json` 展开候选时会原样返回，只在 bmclapi 上重试）。
fn neoforge_version_list_url(mc_version: &str) -> String {
    format!(
        "{}/neoforge/list/{mc_version}",
        Root::McMeta.base(Mirror::Bmclapi)
    )
}

impl DownloadSource {
    /// 取 NeoForge 版本列表，按版本号从新到旧排序。
    pub async fn fetch_neoforge_versions(
        &self,
        mc_version: &str,
    ) -> Result<Vec<NeoForgeListEntry>> {
        let mut listing: Vec<NeoForgeListEntry> = self
            .fetch_json(&neoforge_version_list_url(mc_version))
            .await?;
        listing.sort_by(|a, b| {
            compare(&b.version, &a.version)
                .ok()
                .and_then(Cmp::ord)
                .unwrap_or(Ordering::Equal)
        });
        Ok(listing)
    }
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
    fn neoforge_version_list_url_is_bmclapi_only() {
        let url = neoforge_version_list_url("1.21.1");
        assert_eq!(url, "https://bmclapi2.bangbang93.com/neoforge/list/1.21.1");
        // 不含占位符 ⇒ 不可镜像：解析到官方镜像也原样，不会指向 piston-meta
        assert!(!crate::source::is_templated(&url));
        assert_eq!(crate::source::resolve_url(&url, Mirror::Official), url);
    }
}
