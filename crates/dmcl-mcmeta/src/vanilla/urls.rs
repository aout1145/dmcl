//! 原版的地址与取数（版本清单 / 版本 JSON / 资源索引）。

use anyhow::{anyhow, Result};

use crate::source::{DownloadSource, Root};
use crate::vanilla::asset::AssetIndexFile;
use crate::vanilla::manifest::{VersionManifest, VersionManifestEntry};
use crate::vanilla::version::VersionJson;

/// Minecraft 版本列表。
pub const VERSION_MANIFEST: &str = "${MC_META}/mc/game/version_manifest.json";

/// 带 sha1 的版本列表（v2）。
pub const VERSION_MANIFEST_V2: &str = "${MC_META}/mc/game/version_manifest_v2.json";

/// 资源索引：`${MC_META}/v1/packages/<sha1>/<id>.json`。
pub fn asset_index_url(sha1: &str, id: &str) -> String {
    format!(
        "{}/v1/packages/{sha1}/{id}.json",
        Root::McMeta.placeholder()
    )
}

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
        let mut manifest: VersionManifest = self.get_json(VERSION_MANIFEST).await?;
        manifest.templatize_urls();
        Ok(manifest)
    }

    /// 取带 sha1 的版本列表（v2）。
    pub async fn fetch_version_manifest_v2(&self) -> Result<VersionManifest> {
        let mut manifest: VersionManifest = self.get_json(VERSION_MANIFEST_V2).await?;
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

    /// 取一个版本 JSON。返回的文档里所有 url 都已经是模板形式。
    pub async fn fetch_version_json(&self, url: &str) -> Result<VersionJson> {
        let mut json: VersionJson = self.get_json(url).await?;
        json.templatize_urls();
        Ok(json)
    }

    /// 取资源索引（`assets/indexes/<id>.json` 的远程原文）。
    pub async fn fetch_asset_index(&self, url: &str) -> Result<AssetIndexFile> {
        self.get_json(url).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::Mirror;

    #[test]
    fn urls_are_built_as_templates() {
        assert_eq!(VERSION_MANIFEST, "${MC_META}/mc/game/version_manifest.json");
        assert_eq!(
            asset_index_url("abc", "5"),
            "${MC_META}/v1/packages/abc/5.json"
        );
        assert_eq!(asset_object_url("abcdef"), "${MC_ASSETS}/abcdef");
        assert_eq!(
            maven_library_url("net/sf/jopt/a.jar"),
            "${MC_MAVEN}/net/sf/jopt/a.jar"
        );
    }

    #[test]
    fn resolved_urls_point_at_the_expected_mirror() {
        assert_eq!(
            DownloadSource::official().resolve(VERSION_MANIFEST),
            "https://piston-meta.mojang.com/mc/game/version_manifest.json"
        );
        assert_eq!(
            DownloadSource::new(Mirror::Bmclapi).resolve(asset_index_url("a", "5")),
            "https://bmclapi2.bangbang93.com/v1/packages/a/5.json"
        );
        assert_eq!(
            DownloadSource::new(Mirror::Bmclapi).resolve(maven_library_url("a.jar")),
            "https://bmclapi2.bangbang93.com/maven/a.jar"
        );
    }
}
