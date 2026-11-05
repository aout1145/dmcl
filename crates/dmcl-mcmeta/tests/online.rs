//! 联网冒烟测试（默认 `#[ignore]`）：
//!
//! ```text
//! cargo test -p dmcl-mcmeta --test online -- --ignored --nocapture
//! ```

use dmcl_mcmeta::source::{DownloadSource, Mirror};
use dmcl_mcmeta::vanilla::{current_jre_platform, VersionJson};

#[tokio::test]
#[ignore = "requires network access"]
async fn version_manifest_is_fetched_and_templatized() {
    let source = DownloadSource::official();
    let manifest = source.fetch_version_manifest().await.expect("manifest");

    assert!(manifest.versions.len() > 10);
    assert!(manifest.latest_release().is_some());
    // 清单里的版本 JSON 地址已经归一成模板
    for entry in &manifest.versions {
        assert!(
            entry.url.starts_with("${MC_META}"),
            "未被模板化: {}",
            entry.url
        );
        // 模板能展开成可用的候选地址
        assert!(source
            .candidates(&entry.url)
            .iter()
            .all(|url| url.starts_with("https://")));
    }
}

#[tokio::test]
#[ignore = "requires network access"]
async fn a_version_json_round_trips_through_templates() {
    let source = DownloadSource::official();
    let version: VersionJson = source
        .fetch_version_json_by_id("1.7.10")
        .await
        .expect("version json");

    assert_eq!(version.id.as_deref(), Some("1.7.10"));
    assert!(!version.libraries.is_empty());

    // 官方下载地址已经变成模板……
    let templated = serde_json::to_value(&version).unwrap();
    let template = templated["libraries"][0]["downloads"]["artifact"]["url"]
        .as_str()
        .expect("artifact url");
    assert!(template.starts_with("${MC_"));

    // ……按镜像展开的候选地址与官方不同（说明镜像确实生效）
    let official = DownloadSource::official().candidates(template);
    let mirrored = DownloadSource::new(Mirror::Bmclapi).candidates(template);
    assert_ne!(official[0], mirrored[0]);
}

#[tokio::test]
#[ignore = "requires network access"]
async fn fabric_loader_list_can_be_queried() {
    let source = DownloadSource::official();
    let loaders = source.fetch_fabric_loaders("1.20.1").await.expect("fabric");
    assert!(!loaders.is_empty());
    assert!(loaders.iter().any(|entry| entry.loader.is_stable()));
    // launcherMeta 里属于 Fabric 仓库的库地址已经归一成模板；
    // 其他第三方仓库（如 Maven Central）不属于任何下载根，保持原样。
    let urls: Vec<&str> = loaders
        .iter()
        .flat_map(|entry| {
            let libraries = &entry.launcher_meta.libraries;
            libraries
                .client
                .iter()
                .chain(&libraries.common)
                .chain(&libraries.server)
                .chain(&libraries.development)
        })
        .flat_map(|library| {
            [
                library.url.as_deref(),
                library
                    .downloads
                    .as_ref()
                    .and_then(|downloads| downloads.artifact.as_ref())
                    .and_then(|artifact| artifact.url.as_deref()),
            ]
        })
        .flatten()
        .collect();
    assert!(
        urls.iter().any(|url| url.starts_with("${FABRIC_")),
        "{urls:?}"
    );
    assert!(
        urls.iter().all(|url| !url.contains("fabricmc.net")),
        "{urls:?}"
    );
}

#[tokio::test]
#[ignore = "requires network access"]
async fn optifine_list_and_latest_can_be_queried() {
    // 列表与下载都固定走 bmclapi，与构造 DownloadSource 时选的镜像无关。
    let source = DownloadSource::official();
    let versions = source
        .fetch_optifine_versions("1.19")
        .await
        .expect("optifine");
    assert!(versions.iter().any(|entry| !entry.is_preview()));
    assert!(versions.iter().any(|entry| entry.is_preview()));
    assert!(versions.iter().all(|entry| entry
        .download_url()
        .starts_with("https://bmclapi2.bangbang93.com/optifine/")));

    let latest = source
        .latest_optifine_version("1.19")
        .await
        .expect("latest optifine");
    assert!(!latest.is_empty());
    assert_eq!(
        versions
            .iter()
            .find(|entry| entry.version() == latest)
            .map(|entry| entry.is_preview()),
        Some(false)
    );
}

#[tokio::test]
#[ignore = "requires network access"]
async fn jre_manifest_points_at_a_runtime_for_the_current_platform() {
    let source = DownloadSource::official();
    let manifest = source.fetch_jre_manifest().await.expect("jre manifest");
    assert!(!manifest.platforms.is_empty());

    let platform = current_jre_platform().expect("当前平台应有 Mojang 提供的运行时");
    let entry = manifest
        .runtime("java-runtime-gamma", platform)
        .expect("java-runtime-gamma");
    let url = entry.manifest.as_ref().expect("manifest 地址").url.clone();
    assert!(url.starts_with("${MC_META}/"));

    let runtime = source
        .fetch_jre_runtime_manifest(&url)
        .await
        .expect("runtime manifest");
    assert!(runtime
        .files
        .values()
        .any(|file| file.executable == Some(true)));
    for file in runtime.files.values() {
        let Some(downloads) = &file.downloads else {
            continue;
        };
        for artifact in [&downloads.raw, &downloads.lzma].into_iter().flatten() {
            let url = artifact.url.as_deref().unwrap_or_default();
            assert!(url.starts_with("${MC_DATA}/"), "未被模板化: {url}");
        }
    }
}
