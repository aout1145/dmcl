//! NeoForge installation (was `dmcl.download.neoforge`).

use std::io::{Cursor, Read};
use std::path::Path;

use anyhow::Result;

use super::mirrors::{neoforge_maven, Mirror};
use super::multitasks::base_name;
use super::processors::run_installer_processors;
use crate::mcjson::{InstallProfile, NeoForgeListEntry, NeoForgeVersionListing, VersionJson};
use crate::utils::{
    get_download, get_json_as, version_json_path, write_json_safe, write_safe, zip_has_entry,
    zip_read_entry,
};

/// `officialGetNeoForge`.
pub async fn official_get_neoforge(mc_version: &str) -> Result<Vec<String>> {
    let url = if mc_version == "1.20.1" {
        "https://maven.neoforged.net/api/maven/versions/releases/net/neoforged/forge"
    } else {
        "https://maven.neoforged.net/api/maven/versions/releases/net/neoforged/neoforge"
    };
    let prefix = if mc_version == "1.20.1" {
        "1.20.1"
    } else {
        mc_version.get(2..).unwrap_or("")
    };
    let listing: NeoForgeVersionListing = get_json_as(url).await?;
    Ok(listing
        .versions
        .into_iter()
        .filter(|ver| ver.starts_with(prefix))
        .collect())
}

/// `bmclapiGetNeoForge`.
pub async fn bmclapi_get_neoforge(mc_version: &str) -> Result<Vec<String>> {
    let list: Vec<NeoForgeListEntry> = get_json_as(&format!(
        "https://bmclapi2.bangbang93.com/neoforge/list/{mc_version}"
    ))
    .await?;
    Ok(list.into_iter().map(|entry| entry.version).collect())
}

/// The D `final switch (mirror)` — with [`Mirror`] as an enum the compiler now
/// guarantees that every case is handled (the original threw a `SwitchError`
/// for an unknown mirror).
async fn list_for_mirror(mirror: Mirror, mc_version: &str) -> Result<Vec<String>> {
    Ok(match mirror {
        Mirror::Official => official_get_neoforge(mc_version).await?,
        Mirror::Bmclapi => bmclapi_get_neoforge(mc_version).await?,
    })
}

/// `showNeoForgeVersionList`.
pub async fn show_neoforge_version_list(mirror: Mirror, mc_version: &str) -> Result<()> {
    for ver in list_for_mirror(mirror, mc_version).await? {
        println!("{ver}");
    }
    Ok(())
}

/// `getLatestNeoForge` — the original takes the *last* element of the list.
pub async fn get_latest_neoforge(mirror: Mirror, mc_version: &str) -> Result<String> {
    let list = list_for_mirror(mirror, mc_version).await?;
    Ok(list.last().cloned().unwrap_or_default())
}

/// `installNeoForge`.
pub async fn install_neoforge(
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
    let neoname = if forgever.starts_with("1.20.1") {
        "forge"
    } else {
        "neoforge"
    };
    let installer = get_download(
        &format!(
            "{}/net/neoforged/{neoname}/{forgever}/{neoname}-{forgever}-installer.jar",
            neoforge_maven(mirror)
        ),
        None,
    )
    .await?;
    println!("forge-{forgever}-installer.jar");
    let mut zip = zip::ZipArchive::new(Cursor::new(installer))?;

    // extract neoforge jar
    if zip_has_entry(&mut zip, "maven/") {
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
    }

    // libraries + client.lzma + processors (no `client.lzma` guard here)
    let profile: InstallProfile =
        serde_json::from_slice(&zip_read_entry(&mut zip, "install_profile.json")?)?;
    run_installer_processors(
        mirror, root_path, vername, forgever, &profile, &mut zip, &mc_json,
    )
    .await?;

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
