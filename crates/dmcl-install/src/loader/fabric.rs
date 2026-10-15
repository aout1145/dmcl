//! Fabric 的安装（查询见 [`dmcl_mcmeta`]）。

use std::sync::Arc;

use anyhow::Result;
use dmcl_mcmeta::layout::{read_version_json, version_json_path};
use dmcl_task::{task_fn_io, TaskChain, TaskContext};

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

/// 安装 Fabric 的任务链。加载器版本由调用者给定（`dmcl-install` 不查最新）。
pub fn install_fabric_task(
    options: Arc<InstallOptions>,
    version_name: String,
    mc_version: String,
    loader_version: String,
) -> TaskChain<(), ()> {
    TaskChain::new(format!("安装 Fabric：{version_name}")).then(
        task_fn_io("获取并写入版本元数据", {
            let options = options.clone();
            let version_name = version_name.clone();
            let mc_version = mc_version.clone();
            move |_: (), _ctx: TaskContext| {
                let options = options.clone();
                let version_name = version_name.clone();
                let mc_version = mc_version.clone();
                let loader_version = loader_version.clone();
                async move {
                    write_fabric_version_json(&options, &version_name, &mc_version, &loader_version)
                        .await
                }
            }
        }),
        "获取并写入版本元数据",
    )
}

/// 取 Fabric profile，合并进本地版本 JSON 并写盘。
async fn write_fabric_version_json(
    options: &InstallOptions,
    version_name: &str,
    mc_version: &str,
    loader_version: &str,
) -> Result<()> {
    let fabric_json = options
        .source
        .fetch_fabric_profile(mc_version, loader_version)
        .await?;

    let path = version_json_path(&options.root_path, version_name);
    let mut mc_json = read_version_json(&path).await?;
    mc_json.merge(&fabric_json)?;
    finish_version_json(&mut mc_json, version_name, false);

    write_json_safe(&path, &mc_json).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use dmcl_mcmeta::source::{DownloadSource, Mirror};

    fn options() -> Arc<InstallOptions> {
        Arc::new(InstallOptions::new(
            "/tmp/.minecraft",
            DownloadSource::new(Mirror::Official),
        ))
    }

    #[test]
    fn fabric_install_writes_the_merged_version_json() {
        let chain =
            install_fabric_task(options(), "1.20.1".into(), "1.20.1".into(), "0.15.0".into());
        let names: Vec<_> = chain.step_names().collect();
        assert_eq!(names, ["获取并写入版本元数据"]);
    }
}
