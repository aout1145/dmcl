//! Vanilla game files: version list, libraries and assets
//! (was `dmcl.download.game`).

use std::path::Path;

use anyhow::{anyhow, Result};

use super::mirrors::{mc_maven, mc_meta, mirror_url, Mirror, MC_ASSETS, MC_MAVEN, MC_META};
use super::multitasks::{base_name, download_file, wait_downloads, DownloadFileMeta};
use crate::config::with_config;
use crate::env::LAUNCHER_PROFILES_JSON;
use crate::mcjson::{rules_allow, AssetIndexFile, VersionJson, VersionManifest};
use crate::utils::{
    check_file, download_safe, get_json_as, get_os_name, lib_name_to_path, read_version_json_safe,
    version_jar_path, version_json_path, write_json_safe, write_safe,
};

/// `showVersionList`.
pub async fn show_version_list(mirror: Mirror, types: &[String]) -> Result<()> {
    let manifest: VersionManifest = get_json_as(&format!(
        "{}/mc/game/version_manifest.json",
        mc_meta(mirror)
    ))
    .await?;
    for ver in manifest.versions.iter().rev() {
        if types.iter().any(|t| t == &ver.kind) {
            println!("{} {}", ver.kind, ver.id);
        }
    }
    Ok(())
}

/// `generateLauncherProfiles`.
pub async fn generate_launcher_profiles(root_path: &Path) -> Result<()> {
    let path = root_path.join("launcher_profiles.json");
    if tokio::fs::metadata(&path).await.is_err() {
        write_safe(&path, LAUNCHER_PROFILES_JSON.as_bytes()).await?;
    }
    Ok(())
}

/// `downloadVanilla` — resolve `version_id` in the manifest and store the
/// version json under `version_name`.
pub async fn download_vanilla(
    mirror: Mirror,
    root_path: &Path,
    version_id: &str,
    version_name: &str,
) -> Result<()> {
    generate_launcher_profiles(root_path).await?;

    let manifest: VersionManifest = get_json_as(&format!(
        "{}/mc/game/version_manifest.json",
        mc_meta(mirror)
    ))
    .await?;
    let version_url = manifest
        .versions
        .iter()
        .rev()
        .find(|ver| ver.id == version_id)
        .map(|ver| ver.url.clone())
        .ok_or_else(|| anyhow!("cannot find {version_id}"))?;

    println!("{version_name}.json");
    let mut json: VersionJson =
        get_json_as(&mirror_url(mirror, Some(MC_META), &version_url)).await?;
    json.id = Some(version_name.to_string());
    write_json_safe(version_json_path(root_path, version_name), &json).await
}

/// `downloadLibraries`.
pub async fn download_libraries(
    mirror: Mirror,
    root_path: &Path,
    version_name: &str,
) -> Result<()> {
    let json = read_version_json_safe(root_path, version_name).await?;
    let libraries_dir = root_path.join("libraries");

    // client jar
    if let Some(client) = json.downloads.as_ref().and_then(|d| d.client.as_ref()) {
        download_file(DownloadFileMeta::new(
            mirror_url(mirror, Some(MC_MAVEN), client.url.as_deref().unwrap_or("")),
            version_jar_path(root_path, version_name),
            client.sha1.clone(),
        ));
    }

    for lib in &json.libraries {
        if !rules_allow(&lib.rules) {
            continue;
        }
        let coord = lib.name.as_str();
        let lib_path = lib_name_to_path(coord);

        if let Some(downloads) = &lib.downloads {
            // easy-download libs
            if let Some(artifact) = downloads.artifact.as_ref().filter(|a| a.url.is_some()) {
                download_file(DownloadFileMeta::new(
                    mirror_url(mirror, None, artifact.url.as_deref().unwrap_or("")),
                    libraries_dir.join(artifact.path.as_deref().unwrap_or("")),
                    artifact.sha1.clone(),
                ));
            }
        } else if lib.url.is_none() {
            // difficult-download libs without a repository: use the maven mirror
            download_file(DownloadFileMeta::new(
                format!("{}/{}", mc_maven(mirror), lib_path),
                libraries_dir.join(&lib_path),
                lib.sha1.clone(),
            ));
        } else {
            if coord.starts_with("net.minecraftforge:forge:") {
                // this fucking file cannot be downloaded
                continue;
            }
            download_file(DownloadFileMeta::new(
                mirror_url(
                    mirror,
                    None,
                    &format!("{}/{}", lib.url.as_deref().unwrap_or(""), lib_path),
                ),
                libraries_dir.join(&lib_path),
                lib.sha1.clone(),
            ));
        }

        // native libs
        let native = lib
            .natives
            .as_ref()
            .and_then(|natives| natives.get(get_os_name()))
            .and_then(|native_name| {
                lib.downloads
                    .as_ref()
                    .and_then(|d| d.classifiers.as_ref())
                    .and_then(|classifiers| classifiers.get(native_name))
            });
        if let Some(native) = native {
            download_file(DownloadFileMeta::new(
                mirror_url(mirror, Some(MC_MAVEN), native.url.as_deref().unwrap_or("")),
                libraries_dir.join(native.path.as_deref().unwrap_or("")),
                native.sha1.clone(),
            ));
        }
    }
    Ok(())
}

/// `downloadAssets`.
pub async fn download_assets(mirror: Mirror, root_path: &Path, version_name: &str) -> Result<()> {
    let verjson = read_version_json_safe(root_path, version_name).await?;
    let asset_index = verjson.asset_index.as_ref();
    let id = asset_index.and_then(|a| a.id.clone()).unwrap_or_default();
    let expected_sha1 = asset_index.and_then(|a| a.sha1.clone());
    let idx_url = asset_index.and_then(|a| a.url.clone()).unwrap_or_default();
    let assets_dir = root_path.join("assets");
    let idx_path = assets_dir.join("indexes").join(format!("{id}.json"));

    if !check_file(&idx_path, expected_sha1.as_deref()).await {
        download_safe(&idx_url, &idx_path, expected_sha1.as_deref()).await?;
        println!("{}", base_name(&idx_path));
    }

    let text = tokio::fs::read_to_string(&idx_path).await?;
    let index: AssetIndexFile = serde_json::from_str(&text)?;
    let skip_music = with_config(|c| c.download.i_dont_need_music);

    for (key, asset) in &index.objects {
        if skip_music
            && Path::new(key)
                .extension()
                .map(|e| e == "ogg")
                .unwrap_or(false)
        {
            continue;
        }
        let hash = asset.hash.as_str();
        if hash.len() < 2 {
            continue;
        }
        let path = format!("{}/{}", &hash[0..2], hash);
        let url = mirror_url(
            mirror,
            Some(MC_ASSETS),
            &format!("https://resources.download.minecraft.net/{path}"),
        );
        if index.is_virtual() {
            // 1.6 legacy
            download_file(DownloadFileMeta::new(
                url,
                assets_dir.join("virtual").join(&id).join(key),
                Some(hash.to_string()),
            ));
        } else {
            // others
            download_file(DownloadFileMeta::new(
                url,
                assets_dir.join("objects").join(&path),
                Some(hash.to_string()),
            ));
        }
    }
    Ok(())
}

/// `downloadGameFiles`.
pub async fn download_game_files(
    mirror: Mirror,
    root_path: &Path,
    version_name: &str,
) -> Result<()> {
    download_libraries(mirror, root_path, version_name).await?;
    download_assets(mirror, root_path, version_name).await?;
    wait_downloads().await;
    Ok(())
}
