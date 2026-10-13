//! 原版的地址与取数（版本清单 / 版本 JSON / 资源索引）。

use anyhow::{anyhow, Result};

use crate::source::{DownloadSource, Root};
use crate::vanilla::manifest::{VersionManifest, VersionManifestEntry};
use crate::vanilla::version::VersionJson;

/// 一个资源对象：`${MC_ASSETS}/<hash 前两位>/<hash>`。
pub fn asset_object_url(hash: &str) -> String {
    format!("{}/{hash}", Root::McAssets.placeholder())
}

/// 一个 maven 库：`${MC_MAVEN}/<仓库相对路径>`。
pub fn maven_library_url(path: &str) -> String {
    format!(
        "{}/{}",
        Root::McMaven.placeholder(),
        path.trim_start_matches('/')
    )
}

impl DownloadSource {
    /// 取 Minecraft 版本列表。
    pub async fn fetch_version_manifest(&self) -> Result<VersionManifest> {
        let mut manifest: VersionManifest = self
            .fetch_json("${MC_META}/mc/game/version_manifest.json")
            .await?;
        manifest.templatize_urls();
        Ok(manifest)
    }

    /// 在版本列表里找出某个版本。
    pub async fn find_version(&self, version_id: &str) -> Result<VersionManifestEntry> {
        let manifest = self.fetch_version_manifest().await?;
        manifest
            .find(version_id)
            .cloned()
            .ok_or_else(|| anyhow!("在版本列表里找不到 {version_id}"))
    }

    /// 按版本号取该版本的版本 JSON（内部先查版本列表拿到地址）。
    pub async fn fetch_version_json_by_id(&self, version_id: &str) -> Result<VersionJson> {
        let entry = self.find_version(version_id).await?;
        self.fetch_version_json(&entry.url).await
    }

    /// 取一个版本 JSON（`url` 是清单里的模板地址）。返回的文档里所有 url 都已经是模板形式。
    pub(crate) async fn fetch_version_json(&self, url: &str) -> Result<VersionJson> {
        let mut json: VersionJson = self.fetch_json(url).await?;
        json.templatize_urls();
        Ok(json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::Mirror;

    #[test]
    fn urls_are_built_as_templates() {
        assert_eq!(asset_object_url("abcdef"), "${MC_ASSETS}/abcdef");
        assert_eq!(
            maven_library_url("net/sf/jopt/a.jar"),
            "${MC_MAVEN}/net/sf/jopt/a.jar"
        );
    }

    #[test]
    fn resolved_urls_point_at_the_expected_mirror() {
        assert_eq!(
            DownloadSource::new(Mirror::Bmclapi).resolve(maven_library_url("a.jar")),
            "https://bmclapi2.bangbang93.com/maven/a.jar"
        );
    }
}
