//! Fabric 的安装（查询见 [`dmcl_mcmeta`]）。

use anyhow::Result;
use dmcl_game::ResolvedVersion;
use dmcl_mcmeta::layout::version_json_path;

use crate::io::write_json_safe;
use crate::loader::common::finish_version_json;
use crate::options::InstallOptions;

/// Fabric 的自动安装：把 loader profile 合并进上一步的版本描述并写盘。
pub async fn install_fabric(
    previous: &ResolvedVersion,
    options: &InstallOptions,
    version_name: &str,
    mc_version: &str,
    loader_version: &str,
) -> Result<ResolvedVersion> {
    let fabric_json = options
        .game
        .source
        .fetch_fabric_profile(mc_version, loader_version)
        .await?;

    let mut json = previous.json.clone();
    json.merge(&fabric_json)?;
    finish_version_json(&mut json, version_name, false);
    write_json_safe(
        &version_json_path(&previous.reference.root, version_name),
        &json,
    )
    .await?;

    Ok(ResolvedVersion {
        reference: previous.reference.clone(),
        json,
    })
}
