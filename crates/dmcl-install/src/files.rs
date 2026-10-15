//! 把规划好的文件真正下载下来，并解压 natives。

use std::sync::Arc;

use anyhow::{Context, Result};
use dmcl_download::DownloadReport;
use dmcl_mcmeta::layout::{natives_dir, read_version_json_safe};
use dmcl_mcmeta::os::{Arch, Os};
use dmcl_mcmeta::vanilla::{AssetIndexFile, VersionJson};
use dmcl_task::{task_fn, TaskChain, TaskContext};

use crate::options::InstallOptions;
use crate::plan::{
    plan_asset_objects, plan_assets_index, plan_client_jar, plan_libraries, plan_native_jars,
    plan_natives, PlannedFile,
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
    let json = read_version_json_safe(&options.root_path, version_name).await?;
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

/// 把 natives jar 解压到该版本、该架构的临时目录，返回解压了几个。
///
/// 解压到的目录与启动时引用的是同一个（[`dmcl_mcmeta::layout::natives_dir`]），
/// 目录名带上 os / arch，换平台或换 JVM 架构不会混用。
pub async fn extract_natives(
    options: &InstallOptions,
    version_name: &str,
    os: Os,
    arch: Arch,
    ctx: &TaskContext,
) -> Result<usize> {
    let json = read_version_json_safe(&options.root_path, version_name).await?;
    let target = natives_dir(version_name, os, arch);
    let mut extracted = 0;

    for (jar, name) in plan_natives(&options.plan(), &json, os, arch) {
        if !jar.is_file() {
            // 库文件缺失时启动流程会在完整性检查里发现，这里不当作错误
            ctx.log(format!("跳过缺失的 natives: {name}"));
            continue;
        }
        ctx.log(format!("解压 {name}"));
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
            let json = read_version_json_safe(&options.root_path, &version_name).await?;
            let mut planned = Vec::new();
            if let Some(client) = plan_client_jar(&options.plan(), &json, &version_name) {
                planned.push(client);
            }
            download_planned(&options, "客户端 jar", planned, &ctx).await?;
            Ok(())
        }
    })
}

/// 下载游戏文件的任务链：索引 → 库 → 资源。
///
/// natives 不在这里：它依赖启动时选中的 java 架构，由 [`natives_task`] 单独跑。
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
        .then(assets_step(options, version_name), "下载资源")
}

/// natives 的下载 + 解压任务链。
///
/// 独立于 [`game_files_task`]：natives 要按**启动时选中的 java** 的 os / arch 挑分类器，
/// 因此由启动流程在选好 java 之后再跑。
pub fn natives_task(
    options: Arc<InstallOptions>,
    version_name: String,
    os: Os,
    arch: Arch,
) -> TaskChain<(), ()> {
    TaskChain::new(format!("准备 natives：{version_name}"))
        .then(
            natives_download_step(options.clone(), version_name.clone(), os, arch),
            "下载 natives",
        )
        .then(
            natives_extract_step(options, version_name, os, arch),
            "解压 natives",
        )
}

fn index_step(
    options: Arc<InstallOptions>,
    version_name: String,
) -> impl dmcl_task::Task<(), ()> + 'static {
    task_fn("获取资源索引", move |ctx| {
        let options = options.clone();
        let version_name = version_name.clone();
        async move {
            let json = read_version_json_safe(&options.root_path, &version_name).await?;
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
            let json = read_version_json_safe(&options.root_path, &version_name).await?;
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
            let json = read_version_json_safe(&options.root_path, &version_name).await?;
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

fn natives_download_step(
    options: Arc<InstallOptions>,
    version_name: String,
    os: Os,
    arch: Arch,
) -> impl dmcl_task::Task<(), ()> + 'static {
    task_fn("下载 natives", move |ctx| {
        let options = options.clone();
        let version_name = version_name.clone();
        async move {
            let json = read_version_json_safe(&options.root_path, &version_name).await?;
            let planned = plan_native_jars(&options.plan(), &json, os, arch);
            download_planned(&options, "natives", planned, &ctx).await?;
            Ok(())
        }
    })
}

fn natives_extract_step(
    options: Arc<InstallOptions>,
    version_name: String,
    os: Os,
    arch: Arch,
) -> impl dmcl_task::Task<(), ()> + 'static {
    task_fn("解压 natives", move |ctx| {
        let options = options.clone();
        let version_name = version_name.clone();
        async move {
            extract_natives(&options, &version_name, os, arch, &ctx).await?;
            Ok(())
        }
    })
}
