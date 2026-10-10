//! 把规划好的文件真正下载下来，并解压 natives。

use std::sync::Arc;

use anyhow::{Context, Result};
use dmcl_download::DownloadReport;
use dmcl_mcmeta::{natives_dir, AssetIndexFile, VersionJson};
use dmcl_task::{task_fn, TaskChain, TaskContext};

use crate::options::InstallOptions;
use crate::plan::{
    plan_asset_objects, plan_assets_index, plan_client_jar, plan_libraries, plan_natives,
    PlannedFile,
};
use crate::zip::extract_zip;

/// 跑一批规划好的下载，并检查结果。
///
/// 只要有文件失败就返回错误 —— 安装不能"少几个文件也当成功"。
pub async fn download_planned(
    options: &InstallOptions,
    name: &str,
    planned: Vec<PlannedFile>,
    ctx: &TaskContext,
) -> Result<DownloadReport> {
    if planned.is_empty() {
        return Ok(DownloadReport::default());
    }
    let items = planned
        .into_iter()
        .map(|file| file.into_item(options))
        .collect();
    let task = options.download_task(name, items);
    let report = task.execute(ctx).await?;
    if !report.is_success() {
        anyhow::bail!(
            "{name}: {} 个文件下载失败：\n{}",
            report.failed.len(),
            report.failure_summary(5)
        );
    }
    Ok(report)
}

/// 确保资源索引已经就位，并解析出来。
///
/// 版本没有 `assetIndex` 时返回 `None`。
pub async fn ensure_asset_index(
    options: &InstallOptions,
    json: &VersionJson,
    ctx: &TaskContext,
) -> Result<Option<AssetIndexFile>> {
    let Some(planned) = plan_assets_index(&options.plan(), json) else {
        return Ok(None);
    };
    let path = planned.save_to.clone();
    download_planned(options, "资源索引", vec![planned], ctx).await?;

    let text = tokio::fs::read_to_string(&path)
        .await
        .with_context(|| format!("无法读取资源索引 {}", path.display()))?;
    let index = serde_json::from_str(&text)
        .with_context(|| format!("无法解析资源索引 {}", path.display()))?;
    Ok(Some(index))
}

/// 下载一个版本需要的全部文件（客户端 jar、库、资源），**不含** natives 解压。
pub async fn download_game_files(
    options: &InstallOptions,
    version_name: &str,
    ctx: &TaskContext,
) -> Result<DownloadReport> {
    let json = options.version_json(version_name).await?;
    let index = ensure_asset_index(options, &json, ctx).await?;

    let mut planned = Vec::new();
    if let Some(client) = plan_client_jar(&options.plan(), &json, version_name) {
        planned.push(client);
    }
    planned.extend(plan_libraries(&options.plan(), &json));
    if let Some(index) = &index {
        planned.extend(plan_asset_objects(&options.plan(), &json, index));
    }

    download_planned(options, "游戏文件", planned, ctx).await
}

/// 把 natives jar 解压到该版本的临时目录，返回解压了几个。
///
/// 这个目录在启动时由 [`dmcl_mcmeta::natives_dir`] 给出，启动流程不再自己解压。
pub async fn extract_natives(
    options: &InstallOptions,
    version_name: &str,
    ctx: &TaskContext,
) -> Result<usize> {
    let json = options.version_json(version_name).await?;
    let target = natives_dir(version_name);
    let mut extracted = 0;

    for (jar, name) in plan_natives(&options.plan(), &json) {
        if !jar.is_file() {
            // 库文件缺失时启动流程会在完整性检查里发现，这里不当作错误
            ctx.status(format!("跳过缺失的 natives: {name}"));
            continue;
        }
        ctx.status(format!("解压 {name}"));
        extract_zip(jar, target.clone()).await?;
        extracted += 1;
    }
    Ok(extracted)
}

/// 只下载客户端 jar（版本 jar）。
///
/// 加载器安装（Forge / NeoForge 的 processor）会把 `{MINECRAFT_JAR}` 解析成
/// 这个文件、读它来生成 patched 的客户端 jar，所以它必须先于加载器就位。
pub fn client_jar_task(
    options: Arc<InstallOptions>,
    version_name: String,
) -> impl dmcl_task::Task<(), ()> + 'static {
    task_fn("下载客户端 jar", move |ctx| {
        let options = options.clone();
        let version_name = version_name.clone();
        async move {
            let json = options.version_json(&version_name).await?;
            let mut planned = Vec::new();
            if let Some(client) = plan_client_jar(&options.plan(), &json, &version_name) {
                planned.push(client);
            }
            download_planned(&options, "客户端 jar", planned, &ctx).await?;
            Ok(())
        }
    })
}

/// 下载游戏文件的任务链：索引 → 库 → 资源 → natives。
///
/// 每一步单独读一次本地的版本 JSON（很便宜），因此步骤之间不需要共享可变状态。
pub fn game_files_task(options: Arc<InstallOptions>, version_name: String) -> TaskChain<(), ()> {
    TaskChain::new(format!("下载游戏文件：{version_name}"))
        .then(
            index_step(options.clone(), version_name.clone()),
            "获取资源索引",
        )
        .then(
            libraries_step(options.clone(), version_name.clone()),
            "下载库文件",
        )
        .then(
            assets_step(options.clone(), version_name.clone()),
            "下载资源",
        )
        .then(natives_step(options, version_name), "解压 natives")
}

fn index_step(
    options: Arc<InstallOptions>,
    version_name: String,
) -> impl dmcl_task::Task<(), ()> + 'static {
    task_fn("获取资源索引", move |ctx| {
        let options = options.clone();
        let version_name = version_name.clone();
        async move {
            let json = options.version_json(&version_name).await?;
            ensure_asset_index(&options, &json, &ctx).await?;
            Ok(())
        }
    })
}

fn libraries_step(
    options: Arc<InstallOptions>,
    version_name: String,
) -> impl dmcl_task::Task<(), ()> + 'static {
    task_fn("下载库文件", move |ctx| {
        let options = options.clone();
        let version_name = version_name.clone();
        async move {
            let json = options.version_json(&version_name).await?;
            let mut planned = Vec::new();
            if let Some(client) = plan_client_jar(&options.plan(), &json, &version_name) {
                planned.push(client);
            }
            planned.extend(plan_libraries(&options.plan(), &json));
            download_planned(&options, "库文件", planned, &ctx).await?;
            Ok(())
        }
    })
}

fn assets_step(
    options: Arc<InstallOptions>,
    version_name: String,
) -> impl dmcl_task::Task<(), ()> + 'static {
    task_fn("下载资源", move |ctx| {
        let options = options.clone();
        let version_name = version_name.clone();
        async move {
            let json = options.version_json(&version_name).await?;
            // 索引通常已经在上一步下好了，这里再来一次只会被"已是最新"跳过
            let Some(index) = ensure_asset_index(&options, &json, &ctx).await? else {
                return Ok(());
            };
            let planned = plan_asset_objects(&options.plan(), &json, &index);
            download_planned(&options, "资源文件", planned, &ctx).await?;
            Ok(())
        }
    })
}

fn natives_step(
    options: Arc<InstallOptions>,
    version_name: String,
) -> impl dmcl_task::Task<(), ()> + 'static {
    task_fn("解压 natives", move |ctx| {
        let options = options.clone();
        let version_name = version_name.clone();
        async move {
            extract_natives(&options, &version_name, &ctx).await?;
            Ok(())
        }
    })
}
