//! Fabric 的安装（查询见 [`dmcl_mcmeta`]）。

use std::sync::Arc;

use anyhow::Result;
use dmcl_mcmeta::{latest_fabric_loader, read_version_json, version_json_path};
use dmcl_task::{task_fn_io, TaskChain, TaskContext};

use crate::io::write_json_safe;
use crate::loader::common::finish_version_json;
use crate::options::InstallOptions;

/// `installFabric` —— Fabric 的自动安装非常简单，比 Forge 简单多了（薄封装）。
pub async fn install_fabric(
    options: &InstallOptions,
    version_name: &str,
    mc_version: &str,
    loader_version: &str,
) -> Result<()> {
    write_fabric_version_json(options, version_name, mc_version, loader_version).await
}

/// 解析 Fabric loader 版本（`requested` 为 `None` 时取最新稳定版）。
async fn resolve_fabric_version(
    options: &InstallOptions,
    mc_version: &str,
    requested: Option<&str>,
    ctx: &TaskContext,
) -> Result<String> {
    let loader = match requested {
        Some(loader) => loader.to_string(),
        None => {
            ctx.log("查询最新的稳定 Fabric loader");
            latest_fabric_loader(&options.source, mc_version).await?
        }
    };
    ctx.log(format!("使用 {mc_version}/{loader}"));
    Ok(loader)
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

/// 安装 Fabric 的任务链（`loader_version = None` 时自动取最新稳定版）。
pub fn install_fabric_task(
    options: Arc<InstallOptions>,
    version_name: String,
    mc_version: String,
    loader_version: Option<String>,
) -> TaskChain<(), ()> {
    TaskChain::new(format!("安装 Fabric：{version_name}"))
        .then(
            task_fn_io("确定 Fabric 版本", {
                let options = options.clone();
                let mc_version = mc_version.clone();
                move |_: (), ctx: TaskContext| {
                    let options = options.clone();
                    let mc_version = mc_version.clone();
                    let loader_version = loader_version.clone();
                    async move {
                        resolve_fabric_version(
                            &options,
                            &mc_version,
                            loader_version.as_deref(),
                            &ctx,
                        )
                        .await
                    }
                }
            }),
            "确定 Fabric 版本",
        )
        .then(
            task_fn_io("获取并写入版本元数据", {
                let options = options.clone();
                let version_name = version_name.clone();
                let mc_version = mc_version.clone();
                move |loader: String, _ctx: TaskContext| {
                    let options = options.clone();
                    let version_name = version_name.clone();
                    let mc_version = mc_version.clone();
                    async move {
                        write_fabric_version_json(&options, &version_name, &mc_version, &loader)
                            .await
                    }
                }
            }),
            "获取并写入版本元数据",
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use dmcl_mcmeta::{DownloadSource, Mirror};

    fn options() -> Arc<InstallOptions> {
        Arc::new(InstallOptions::new(
            "/tmp/.minecraft",
            DownloadSource::new(Mirror::Official),
        ))
    }

    #[test]
    fn fabric_install_is_two_named_steps() {
        let chain = install_fabric_task(
            options(),
            "1.20.1".into(),
            "1.20.1".into(),
            Some("0.15.0".into()),
        );
        let names: Vec<_> = chain.step_names().collect();
        assert_eq!(names, ["确定 Fabric 版本", "获取并写入版本元数据"]);
    }
}
