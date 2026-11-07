//! Fabric 的安装（查询见 [`dmcl_mcmeta`]）。

use anyhow::Result;
use dmcl_mcmeta::layout::{read_version_json, version_json_path};

use crate::io::write_json_safe;
use crate::loader::common::finish_version_json;
use crate::options::InstallOptions;

/// Fabric 的自动安装：把 loader profile 合并进原版版本 JSON 即可。
pub async fn install_fabric(
    options: &InstallOptions,
    version_name: &str,
    mc_version: &str,
    loader_version: &str,
) -> Result<()> {
    write_fabric_version_json(options, version_name, mc_version, loader_version).await
}

/// 取 Fabric profile，合并进本地版本 JSON 并写盘。
async fn write_fabric_version_json(
    options: &InstallOptions,
    version_name: &str,
    mc_version: &str,
    loader_version: &str,
) -> Result<()> {
    let fabric_json = options
        .game
        .source
        .fetch_fabric_profile(mc_version, loader_version)
        .await?;

    let path = version_json_path(&options.root_path, version_name);
    let mut mc_json = read_version_json(&path).await?;
    mc_json.merge(&fabric_json)?;
    finish_version_json(&mut mc_json, version_name, false);

    write_json_safe(&path, &mc_json).await
}
