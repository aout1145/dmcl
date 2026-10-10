//! Fabric 的元数据、地址与取数（loader 版本列表、版本 JSON）。

use anyhow::Result;
use serde::Deserialize;

use crate::source::{DownloadSource, Root};
use crate::vanilla::version::VersionJson;

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

/// Fabric 的 loader 版本列表：`${FABRIC_META}/v2/versions/loader/<mc>`。
pub fn fabric_loader_list_url(mc_version: &str) -> String {
    format!(
        "{}/v2/versions/loader/{mc_version}",
        Root::FabricMeta.placeholder()
    )
}

/// Fabric 的版本 JSON：`${FABRIC_META}/v2/versions/loader/<mc>/<loader>/profile/json`。
pub fn fabric_profile_url(mc_version: &str, loader_version: &str) -> String {
    format!(
        "{}/v2/versions/loader/{mc_version}/{loader_version}/profile/json",
        Root::FabricMeta.placeholder()
    )
}

impl DownloadSource {
    /// 取 Fabric 的 loader 版本列表（从新到旧的顺序由调用者决定）。
    pub async fn fetch_fabric_loaders(&self, mc_version: &str) -> Result<Vec<FabricLoaderEntry>> {
        self.get_json(&fabric_loader_list_url(mc_version)).await
    }

    /// 取 Fabric 的版本 JSON（需要合并进原版版本 JSON）。
    pub async fn fetch_fabric_profile(
        &self,
        mc_version: &str,
        loader_version: &str,
    ) -> Result<VersionJson> {
        self.fetch_version_json(&fabric_profile_url(mc_version, loader_version))
            .await
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fabric_urls_are_built_as_templates() {
        assert_eq!(
            fabric_profile_url("1.20.1", "0.15.0"),
            "${FABRIC_META}/v2/versions/loader/1.20.1/0.15.0/profile/json"
        );
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
