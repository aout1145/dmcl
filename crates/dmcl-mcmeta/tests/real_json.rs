//! 用**真实文档**校验模型完整性（默认 `#[ignore]`，需要一个本地文档目录）。
//!
//! ```text
//! DMCL_MCMETA_JSON_DIR=/tmp/opencode/mcjson \
//!   cargo test -p dmcl-mcmeta --test real_json -- --ignored --nocapture
//! ```
//!
//! 目录里应有：
//! * `1.5.2.json` / `1.12.2.json` / `1.16.5.json` / `1.21.1.json` / `26.3.json`
//! * `manifest.json`
//! * `fabric-1.21.1.json`、`fabric-loaders.json`
//! * `installers/forge-*.json`（版本列表）、`installers/nf-list.json`
//! * `installers/ex/<forge>/install_profile.json`、`version.json`

use std::path::{Path, PathBuf};

use dmcl_mcmeta::fabric::FabricLoaderEntry;
use dmcl_mcmeta::forge::{ForgeVersionEntry, InstallProfile};
use dmcl_mcmeta::neoforge::NeoForgeListEntry;
use dmcl_mcmeta::vanilla::{AssetIndexFile, VersionJson, VersionManifest};

/// 允许留在 `extra` 里的"注释/记账"噪声。
const COMMENTS: &[&str] = &["_comment_", "_comment"];

fn dir() -> Option<PathBuf> {
    std::env::var_os("DMCL_MCMETA_JSON_DIR").map(PathBuf::from)
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("读 {} 失败: {e}", path.display()))
}

fn unexpected(extra: &serde_json::Map<String, serde_json::Value>) -> Vec<String> {
    extra
        .keys()
        .filter(|key| !COMMENTS.contains(&key.as_str()))
        .cloned()
        .collect()
}

/// 校验一份版本 JSON（原版 / Fabric / Forge / NeoForge 通用）。
fn check_version(path: &Path) {
    let name = path.display();
    let json: VersionJson = serde_json::from_str(&read(path)).unwrap();
    let top = unexpected(&json.extra);
    assert!(top.is_empty(), "{name} 顶层未建模字段: {top:?}");

    for (i, lib) in json.libraries.iter().enumerate() {
        assert!(
            lib.extra.is_empty(),
            "{name} libraries[{i}] 未建模字段: {:?}",
            lib.extra.keys().collect::<Vec<_>>()
        );
        if let Some(extract) = &lib.extract {
            assert!(
                extract.extra.is_empty(),
                "{name} libraries[{i}].extract 未建模字段"
            );
        }
        if let Some(downloads) = &lib.downloads {
            assert!(
                downloads.extra.is_empty(),
                "{name} libraries[{i}].downloads 未建模字段: {:?}",
                downloads.extra.keys().collect::<Vec<_>>()
            );
            let artifacts = downloads
                .artifact
                .iter()
                .chain(downloads.classifiers.iter().flat_map(|c| c.values()));
            for artifact in artifacts {
                assert!(
                    artifact.extra.is_empty(),
                    "{name} libraries[{i}] artifact 未建模字段: {:?}",
                    artifact.extra.keys().collect::<Vec<_>>()
                );
            }
        }
    }

    if let Some(downloads) = &json.downloads {
        assert!(downloads.extra.is_empty(), "{name} downloads 未建模字段");
        for artifact in downloads.artifacts() {
            assert!(
                artifact.extra.is_empty(),
                "{name} downloads.* 未建模字段: {:?}",
                artifact.extra.keys().collect::<Vec<_>>()
            );
        }
    }

    if let Some(asset_index) = &json.asset_index {
        assert!(asset_index.extra.is_empty(), "{name} assetIndex 未建模字段");
    }
    if let Some(arguments) = &json.arguments {
        assert!(arguments.extra.is_empty(), "{name} arguments 未建模字段");
    }
    if let Some(logging) = &json.logging {
        assert!(logging.extra.is_empty(), "{name} logging 未建模字段");
        if let Some(client) = &logging.client {
            assert!(client.extra.is_empty(), "{name} logging.client 未建模字段");
            if let Some(file) = &client.file {
                assert!(
                    file.extra.is_empty(),
                    "{name} logging.client.file 未建模字段"
                );
            }
        }
    }
    if let Some(java) = &json.java_version {
        assert!(java.extra.is_empty(), "{name} javaVersion 未建模字段");
    }
}

#[test]
#[ignore = "需要 DMCL_MCMETA_JSON_DIR 指向本地真实文档目录"]
fn real_documents_are_fully_modeled() {
    let dir = dir().expect("请设置 DMCL_MCMETA_JSON_DIR");

    // 原版版本 JSON（覆盖 1.5.2 的旧结构到 26.3 的新结构）
    for name in [
        "1.5.2.json",
        "1.12.2.json",
        "1.16.5.json",
        "1.21.1.json",
        "26.3.json",
    ] {
        check_version(&dir.join(name));
    }

    // 版本清单
    let manifest: VersionManifest =
        serde_json::from_str(&read(&dir.join("manifest.json"))).unwrap();
    assert!(manifest.versions.len() > 10);

    // 资源索引：顶层与每个 object 都不该有未建模字段
    let assets_dir = dir.join("assets");
    let mut asset_indexes = 0;
    for entry in std::fs::read_dir(&assets_dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let index: AssetIndexFile = serde_json::from_str(&read(&path)).unwrap();
        let extra = unexpected(&index.extra);
        assert!(extra.is_empty(), "{} 未建模字段: {extra:?}", path.display());
        for (name, object) in &index.objects {
            assert!(
                object.extra.is_empty(),
                "{} objects[{name}] 未建模字段: {:?}",
                path.display(),
                object.extra.keys().collect::<Vec<_>>()
            );
        }
        asset_indexes += 1;
    }
    assert!(asset_indexes >= 5, "至少应校验 5 份资源索引");

    // Fabric：profile（库级校验和）+ loader 列表
    check_version(&dir.join("fabric-1.21.1.json"));
    let loaders: Vec<FabricLoaderEntry> =
        serde_json::from_str(&read(&dir.join("fabric-loaders.json"))).unwrap();
    assert!(!loaders.is_empty());
    assert!(loaders[0].loader.build.is_some());

    // 加载器版本列表
    let forge: Vec<ForgeVersionEntry> =
        serde_json::from_str(&read(&dir.join("installers/forge-1.21.1.json"))).unwrap();
    assert!(forge.iter().all(|e| e.files.is_some()));
    let neo: Vec<NeoForgeListEntry> =
        serde_json::from_str(&read(&dir.join("installers/nf-list.json"))).unwrap();
    assert!(neo.iter().all(|e| e.raw_version.is_some()));

    // Forge / NeoForge 安装器档案 + 内嵌版本 JSON
    for entry in std::fs::read_dir(dir.join("installers/ex")).unwrap() {
        let entry = entry.unwrap();
        if !entry.path().is_dir() {
            continue;
        }
        let profile_path = entry.path().join("install_profile.json");
        if !profile_path.exists() {
            continue;
        }
        let profile: InstallProfile = serde_json::from_str(&read(&profile_path)).unwrap();
        let extra = unexpected(&profile.extra);
        assert!(
            extra.is_empty(),
            "{} 未建模字段: {extra:?}",
            profile_path.display()
        );
        if let Some(install) = &profile.install {
            assert!(
                install.extra.is_empty(),
                "{} install 未建模字段: {:?}",
                profile_path.display(),
                install.extra.keys().collect::<Vec<_>>()
            );
        }
        // 新式安装器带 libraries，经典安装器把库放在 versionInfo 里
        assert!(
            !profile.libraries.is_empty() || profile.version_info.is_some(),
            "{} 既没有 libraries 也没有 versionInfo",
            profile_path.display()
        );

        let version_path = entry.path().join("version.json");
        if version_path.exists() {
            check_version(&version_path);
        }
    }
}
