//! 根元数据：常量、地址构造，以及"按模板发请求"的取数方法。
//!
//! 这里的常量都是 **URL 模板**，例如 [`VERSION_MANIFEST`] 是
//! `${MC_META}/mc/game/version_manifest.json`。调用者把它们交给
//! [`DownloadSource`]，由下载源决定最终落到官方还是镜像地址。
//!
//! 少数接口（Forge 的版本列表等）**只有 bmclapi 提供**，这些地址是写死的绝对
//! url —— 与原实现一致，已逐条注明。

use anyhow::{anyhow, Result};

use crate::asset::AssetIndexFile;
use crate::install_profile::InstallProfile;
use crate::loader::{
    FabricLoaderEntry, ForgeVersionEntry, NeoForgeListEntry, NeoForgeVersionListing,
};
use crate::manifest::{VersionManifest, VersionManifestEntry};
use crate::source::{DownloadSource, Mirror, Root};
use crate::version::VersionJson;

// ---------------------------------------------------------------------------
// 根元数据常量
// ---------------------------------------------------------------------------

/// Minecraft 版本列表。
pub const VERSION_MANIFEST: &str = "${MC_META}/mc/game/version_manifest.json";

/// 带 sha1 的版本列表（v2）。
pub const VERSION_MANIFEST_V2: &str = "${MC_META}/mc/game/version_manifest_v2.json";

/// Forge 的版本列表接口 —— **只有 bmclapi 提供**（原实现同样写死 bmclapi）。
pub const FORGE_VERSION_LIST: &str = "https://bmclapi2.bangbang93.com/forge/minecraft";

/// bmclapi 的 NeoForge 版本列表接口。
pub const NEOFORGE_VERSION_LIST_BMCLAPI: &str = "https://bmclapi2.bangbang93.com/neoforge/list";

// ---------------------------------------------------------------------------
// 地址构造
// ---------------------------------------------------------------------------

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

/// Forge 安装器：`${FORGE_MAVEN}/net/minecraftforge/forge/<ver>/forge-<ver>-installer.jar`。
pub fn forge_installer_url(forge_version: &str) -> String {
    format!(
        "{}/net/minecraftforge/forge/{forge_version}/forge-{forge_version}-installer.jar",
        Root::ForgeMaven.placeholder()
    )
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

/// Forge 版本列表地址（bmclapi only）。
pub fn forge_version_list_url(mc_version: &str) -> String {
    format!("{FORGE_VERSION_LIST}/{mc_version}")
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

// ---------------------------------------------------------------------------
// 取数
// ---------------------------------------------------------------------------

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

    /// 取安装器档案（`install_profile.json`）。返回的文档里 url 已模板化。
    pub async fn fetch_install_profile(&self, url: &str) -> Result<InstallProfile> {
        let mut profile: InstallProfile = self.get_json(url).await?;
        profile.templatize_urls();
        Ok(profile)
    }

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

    /// 取 Forge 版本列表 —— 只有 bmclapi 提供，与原实现一致。
    pub async fn fetch_forge_versions(&self, mc_version: &str) -> Result<Vec<ForgeVersionEntry>> {
        self.get_json(&forge_version_list_url(mc_version)).await
    }

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

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(
            forge_installer_url("1.20.1-47.2.0"),
            "${FORGE_MAVEN}/net/minecraftforge/forge/1.20.1-47.2.0/forge-1.20.1-47.2.0-installer.jar"
        );
        assert_eq!(
            fabric_profile_url("1.20.1", "0.15.0"),
            "${FABRIC_META}/v2/versions/loader/1.20.1/0.15.0/profile/json"
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
    fn the_forge_version_list_is_bmclapi_only() {
        assert_eq!(
            forge_version_list_url("1.20.1"),
            "https://bmclapi2.bangbang93.com/forge/minecraft/1.20.1"
        );
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
