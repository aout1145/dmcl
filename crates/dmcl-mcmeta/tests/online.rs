//! 联网冒烟测试（默认 `#[ignore]`）：
//!
//! ```text
//! cargo test -p dmcl-mcmeta --test online -- --ignored --nocapture
//! ```

use dmcl_mcmeta::source::{DownloadSource, Mirror};
use dmcl_mcmeta::vanilla::{VersionJson, VERSION_MANIFEST};

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
        // 模板能被解析回官方地址
        assert!(source.resolve(&entry.url).starts_with("https://"));
    }
    assert_eq!(
        source.resolve(VERSION_MANIFEST),
        "https://piston-meta.mojang.com/mc/game/version_manifest.json"
    );
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
    assert!(templated["libraries"][0]["downloads"]["artifact"]["url"]
        .as_str()
        .unwrap_or_default()
        .starts_with("${MC_"));

    // ……而解析回官方源之后与镜像源不同（说明镜像确实生效）
    let mirrored = version.clone().resolved(Mirror::Bmclapi);
    assert_ne!(
        serde_json::to_string(&version).unwrap(),
        serde_json::to_string(&mirrored).unwrap()
    );
}

#[tokio::test]
#[ignore = "requires network access"]
async fn fabric_loader_list_can_be_queried() {
    let source = DownloadSource::official();
    let loaders = source.fetch_fabric_loaders("1.20.1").await.expect("fabric");
    assert!(!loaders.is_empty());
    assert!(loaders.iter().any(|entry| entry.loader.is_stable()));
}
