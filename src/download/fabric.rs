//! Fabric installation (was `dmcl.download.fabric`).

use std::path::Path;

use anyhow::Result;

use super::mirrors::{fabric_meta, Mirror};
use crate::mcjson::{FabricLoaderEntry, VersionJson};
use crate::utils::{get_json_as, version_json_path, write_json_safe};

/// `showFabricVersionList`.
pub async fn show_fabric_version_list(mirror: Mirror, mc_version: &str) -> Result<()> {
    let list: Vec<FabricLoaderEntry> = get_json_as(&format!(
        "{}/v2/versions/loader/{mc_version}",
        fabric_meta(mirror)
    ))
    .await?;
    for meta in list.iter().rev() {
        println!(
            "{mc_version}/{}{}",
            meta.loader.version,
            if meta.loader.is_stable() {
                "(recommanded)"
            } else {
                ""
            }
        );
    }
    Ok(())
}

/// `getLatestFabric` — most recent *stable* loader, otherwise the first entry.
pub async fn get_latest_fabric(mirror: Mirror, mc_version: &str) -> Result<String> {
    let list: Vec<FabricLoaderEntry> = get_json_as(&format!(
        "{}/v2/versions/loader/{mc_version}",
        fabric_meta(mirror)
    ))
    .await?;
    for meta in list.iter().rev() {
        if meta.loader.is_stable() {
            return Ok(format!("{mc_version}/{}", meta.loader.version));
        }
    }
    Ok(format!(
        "{mc_version}/{}",
        list.first()
            .map(|m| m.loader.version.as_str())
            .unwrap_or("")
    ))
}

/// `installFabric` — fabric autoinstall is too easy, look at what forge did!!!
pub async fn install_fabric(
    mirror: Mirror,
    root_path: &Path,
    mc_version: &str,
    fabric_version: &str,
) -> Result<()> {
    let fabric_json: VersionJson = get_json_as(&format!(
        "{}/v2/versions/loader/{fabric_version}/profile/json",
        fabric_meta(mirror)
    ))
    .await?;

    let verpath = version_json_path(root_path, mc_version);
    let mut mc_json: VersionJson =
        serde_json::from_str(&tokio::fs::read_to_string(&verpath).await?)?;
    mc_json.merge(&fabric_json)?;
    mc_json.inherits_from = None;
    mc_json.id = Some(mc_version.to_string());

    write_json_safe(&verpath, &mc_json).await
}
