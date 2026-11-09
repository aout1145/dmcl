//! Fabric 的安装（查询见 [`dmcl_mcmeta`]）。

use std::sync::Arc;

use anyhow::Result;
use dmcl_download::DownloadContext;
use dmcl_game::{download_planned, plan_libraries, ResolvedVersion, VersionRef};
use dmcl_mcmeta::json::JsonMap;
use dmcl_mcmeta::vanilla::{VersionJson, VersionJsonPatch};
use dmcl_task::{task_fn, Handle, StepId, TaskContext, TaskGraphBuilder};

use crate::options::InstallOptions;

/// 取 Fabric 的 loader profile。
pub async fn fetch_fabric_metadata(
    options: &InstallOptions,
    mc_version: &str,
    loader_version: &str,
) -> Result<VersionJson> {
    options
        .game
        .source
        .fetch_fabric_profile(mc_version, loader_version)
        .await
}

/// 下载 Fabric loader profile 里的库。
///
/// 这里不碰版本 JSON；真正的合并与写盘在安装图最后的「完成」步骤统一处理。Fabric 库
/// 可以和原版库同时下载，同一个目标路径由下载层串行化。
pub async fn download_fabric_libraries(
    reference: &VersionRef,
    options: &InstallOptions,
    downloads: &Arc<DownloadContext>,
    metadata: &VersionJson,
    ctx: &TaskContext,
) -> Result<()> {
    let planned = plan_libraries(reference, &metadata.libraries);
    download_planned(&options.game, downloads, "Fabric 依赖库", planned, ctx).await
}

/// 把 Fabric loader profile 转成要应用的 patch。
pub fn fabric_patch(loader_version: &str, metadata: &VersionJson) -> VersionJsonPatch {
    VersionJsonPatch {
        name: "fabric".to_string(),
        version: loader_version.to_string(),
        libraries: metadata.libraries.clone(),
        tweakers: Vec::new(),
        main_class: metadata.main_class.clone(),
        kind: metadata.kind.clone(),
        release_time: metadata.release_time.clone(),
        arguments: metadata.arguments.clone(),
        extra: JsonMap::new(),
    }
}

/// 往安装图里追加 Fabric：获取元数据 → 下载依赖库 → 生成 patch。
pub(crate) fn add_fabric_install(
    builder: &mut TaskGraphBuilder,
    step: StepId,
    options: Arc<InstallOptions>,
    downloads: Arc<DownloadContext>,
    base: Handle<ResolvedVersion>,
    mc_version: String,
    loader_version: String,
) -> Handle<VersionJsonPatch> {
    let metadata = builder.add(
        step,
        "获取 Fabric 元数据",
        base.clone(),
        task_fn({
            let options = options.clone();
            let loader_version = loader_version.clone();
            move |_base: Arc<ResolvedVersion>, _ctx: TaskContext| {
                let options = options.clone();
                let mc_version = mc_version.clone();
                let loader_version = loader_version.clone();
                async move { fetch_fabric_metadata(&options, &mc_version, &loader_version).await }
            }
        }),
    );

    let downloaded = builder.add(
        step,
        "下载 Fabric 依赖库",
        (base, metadata.clone()),
        task_fn({
            move |(base, metadata): (Arc<ResolvedVersion>, Arc<VersionJson>), ctx: TaskContext| {
                let options = options.clone();
                let downloads = downloads.clone();
                async move {
                    download_fabric_libraries(
                        &base.reference,
                        &options,
                        &downloads,
                        &metadata,
                        &ctx,
                    )
                    .await
                }
            }
        }),
    );

    builder.add(
        step,
        "生成 patch",
        (metadata, downloaded),
        task_fn(
            move |(metadata, _downloaded): (Arc<VersionJson>, Arc<()>), ctx: TaskContext| {
                let loader_version = loader_version.clone();
                async move {
                    let patch = fabric_patch(&loader_version, &metadata);
                    ctx.log(format!("已生成 Fabric {} patch", patch.version));
                    Ok(patch)
                }
            },
        ),
    )
}
