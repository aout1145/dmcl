//! Forge installation (was `dmcl.download.forge`).

use std::io::{Cursor, Read};
use std::path::Path;

use anyhow::Result;

use super::mirrors::{forge_maven, Mirror};
use super::multitasks::base_name;
use super::processors::run_installer_processors;
use crate::mcjson::{ForgeVersionEntry, InstallProfile, VersionJson};
use crate::utils::{
    get_download, get_json_as, lib_name_to_path, version_json_path, write_json_safe, write_safe,
    zip_has_entry, zip_read_entry,
};

/// `bmclapiGetForgeVersionName`.
pub fn bmclapi_get_forge_version_name(ver: &ForgeVersionEntry) -> String {
    let mut name = format!("{}-{}", ver.mcversion, ver.version);
    if let Some(branch) = ver.branch.as_deref() {
        // `("branch" in ver.object && !ver["branch"].isNull())`
        name.push('-');
        name.push_str(branch);
    }
    name
}

/// `showForgeVersionList` — bmclapi only, exactly like the original.
pub async fn show_forge_version_list(_mirror: Mirror, mcver: &str) -> Result<()> {
    let mut versions: Vec<ForgeVersionEntry> = get_json_as(&format!(
        "https://bmclapi2.bangbang93.com/forge/minecraft/{mcver}"
    ))
    .await?;
    versions.sort_by_key(|v| v.build);
    for ver in &versions {
        println!("{}", bmclapi_get_forge_version_name(ver));
    }
    Ok(())
}

/// `getLatestForge` — bmclapi only, exactly like the original.
pub async fn get_latest_forge(_mirror: Mirror, mcver: &str) -> Result<String> {
    let mut versions: Vec<ForgeVersionEntry> = get_json_as(&format!(
        "https://bmclapi2.bangbang93.com/forge/minecraft/{mcver}"
    ))
    .await?;
    versions.sort_by_key(|v| std::cmp::Reverse(v.build));
    Ok(versions
        .first()
        .map(bmclapi_get_forge_version_name)
        .unwrap_or_default())
}

/// `installForge`.
pub async fn install_forge(
    mirror: Mirror,
    root_path: &Path,
    vername: &str,
    forgever: &str,
) -> Result<()> {
    let verpath = version_json_path(root_path, vername);
    let mut mc_json: VersionJson =
        serde_json::from_str(&tokio::fs::read_to_string(&verpath).await?)?;
    let libraries_dir = root_path.join("libraries");

    // download installer
    let installer = get_download(
        &format!(
            "{}/net/minecraftforge/forge/{forgever}/forge-{forgever}-installer.jar",
            forge_maven(mirror)
        ),
        None,
    )
    .await?;
    println!("forge-{forgever}-installer.jar");
    let mut zip = zip::ZipArchive::new(Cursor::new(installer))?;

    // extract forge jar
    if zip_has_entry(&mut zip, "maven/") {
        // forge jar in path maven
        let count = zip.len();
        let mut files: Vec<(String, Vec<u8>)> = Vec::new();
        for i in 0..count {
            let mut entry = zip.by_index(i)?;
            let name = entry.name().to_string();
            if !name.ends_with('/') && name.starts_with("maven/") {
                let mut buf = Vec::new();
                entry.read_to_end(&mut buf)?;
                files.push((name, buf));
            }
        }
        for (name, buf) in files {
            let target = libraries_dir.join(&name["maven/".len()..]);
            write_safe(&target, &buf).await?;
            println!("{}", base_name(Path::new(&name)));
        }
    } else if !zip_has_entry(&mut zip, "version.json") {
        // forge jar in root
        let forgelibpath = lib_name_to_path(&format!("net.minecraftforge:forge:{forgever}"));
        let profile: InstallProfile =
            serde_json::from_slice(&zip_read_entry(&mut zip, "install_profile.json")?)?;
        // a zip entry name, not a filesystem path
        let jarpath = profile.file_path().unwrap_or("").to_string();
        let buf = zip_read_entry(&mut zip, &jarpath)?;
        write_safe(libraries_dir.join(&forgelibpath), &buf).await?;
        println!("{}", base_name(Path::new(&forgelibpath)));
    }

    if zip_has_entry(&mut zip, "data/client.lzma") {
        // 1.13+ generate forge-client.jar
        let profile: InstallProfile =
            serde_json::from_slice(&zip_read_entry(&mut zip, "install_profile.json")?)?;
        run_installer_processors(
            mirror, root_path, vername, forgever, &profile, &mut zip, &mc_json,
        )
        .await?;
    }

    // generate version json
    let version_json: VersionJson = if zip_has_entry(&mut zip, "version.json") {
        serde_json::from_slice(&zip_read_entry(&mut zip, "version.json")?)?
    } else {
        let profile: InstallProfile =
            serde_json::from_slice(&zip_read_entry(&mut zip, "install_profile.json")?)?;
        let zip2path = profile.file_path().unwrap_or("").to_string();
        let inner = zip_read_entry(&mut zip, &zip2path)?;
        let mut zip2 = zip::ZipArchive::new(Cursor::new(inner))?;
        serde_json::from_slice(&zip_read_entry(&mut zip2, "version.json")?)?
    };

    mc_json.merge(&version_json)?;
    mc_json.extra.remove("_comment_");
    mc_json.inherits_from = None;
    mc_json.id = Some(vername.to_string());
    mc_json.jar = Some(vername.to_string());

    write_json_safe(&verpath, &mc_json).await
}
