//! Fabric / Forge / NeoForge 的版本查询与安装。
//!
//! 查询部分（[`fabric_loaders`]、[`forge_versions`] …）直接转调
//! [`dmcl_mcmeta::DownloadSource`]，把原实现里散落在下载模块里的接口收敛到这里；
//! "取最新版"的规则也与原实现逐条保持一致（见各函数文档）。

use std::io::Cursor;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use dmcl_mcmeta::{
    forge_installer_url, lib_name_to_path, libraries_dir, neoforge_installer_url,
    read_version_json, version_json_path, DownloadSource, FabricLoaderEntry, ForgeVersionEntry,
    InstallProfile, VersionJson,
};
use dmcl_task::{task_fn, TaskChain, TaskContext};

use crate::files::download_planned;
use crate::io::{write_json_safe, write_safe};
use crate::options::InstallOptions;
use crate::plan::{base_name, PlannedFile};
use crate::processors::run_installer_processors;
use crate::zip::{open_archive, zip_has_entry, zip_read_entry};

// ---------------------------------------------------------------------------
// 版本查询
// ---------------------------------------------------------------------------

/// Fabric 的 loader 列表（远端顺序）。
pub async fn fabric_loaders(
    source: &DownloadSource,
    mc_version: &str,
) -> Result<Vec<FabricLoaderEntry>> {
    source.fetch_fabric_loaders(mc_version).await
}

/// 最新的**稳定** Fabric loader；没有稳定版时取列表第一个（与原实现一致）。
pub async fn latest_fabric_loader(source: &DownloadSource, mc_version: &str) -> Result<String> {
    let list = fabric_loaders(source, mc_version).await?;
    for entry in list.iter().rev() {
        if entry.loader.is_stable() {
            return Ok(entry.loader.version.clone());
        }
    }
    Ok(list
        .first()
        .map(|entry| entry.loader.version.clone())
        .unwrap_or_default())
}

/// Forge 版本列表（bmclapi only），按 build 升序。
pub async fn forge_versions(
    source: &DownloadSource,
    mc_version: &str,
) -> Result<Vec<ForgeVersionEntry>> {
    let mut versions = source.fetch_forge_versions(mc_version).await?;
    versions.sort_by_key(|entry| entry.build);
    Ok(versions)
}

/// 最新 Forge（build 号最大）。
pub async fn latest_forge_version(source: &DownloadSource, mc_version: &str) -> Result<String> {
    let versions = forge_versions(source, mc_version).await?;
    Ok(versions
        .iter()
        .max_by_key(|entry| entry.build)
        .map(ForgeVersionEntry::full_name)
        .unwrap_or_default())
}

/// NeoForge 版本列表。
pub async fn neoforge_versions(source: &DownloadSource, mc_version: &str) -> Result<Vec<String>> {
    source.fetch_neoforge_versions(mc_version).await
}

/// 最新 NeoForge —— 原实现取列表的**最后一个**元素。
pub async fn latest_neoforge_version(source: &DownloadSource, mc_version: &str) -> Result<String> {
    let versions = neoforge_versions(source, mc_version).await?;
    Ok(versions.last().cloned().unwrap_or_default())
}

// ---------------------------------------------------------------------------
// 安装
// ---------------------------------------------------------------------------

/// `installFabric` —— Fabric 的自动安装非常简单，比 Forge 简单多了。
pub async fn install_fabric(
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

/// `installForge`。
pub async fn install_forge(
    options: &InstallOptions,
    version_name: &str,
    forge_version: &str,
    ctx: &TaskContext,
) -> Result<()> {
    let path = version_json_path(&options.root_path, version_name);
    let mut mc_json = read_version_json(&path).await?;
    let libraries = libraries_dir(&options.root_path);

    let installer = fetch_installer(
        options,
        forge_installer_url(forge_version),
        format!("forge-{forge_version}-installer.jar"),
        forge_version,
        ctx,
    )
    .await?;
    let mut zip = open_archive(installer)?;

    // 解出安装器里自带的 jar
    if zip_has_entry(&mut zip, "maven/") {
        extract_maven_entries(&mut zip, &libraries, ctx).await?;
    } else if !zip_has_entry(&mut zip, "version.json") {
        // 老版 Forge：安装包根目录下只有一个 install_profile.json
        let profile = read_install_profile(&mut zip)?;
        let entry = profile.file_path().unwrap_or("").to_string();
        let bytes = zip_read_entry(&mut zip, &entry)?;
        let target = libraries.join(lib_name_to_path(&format!(
            "net.minecraftforge:forge:{forge_version}"
        )));
        write_safe(&target, &bytes).await?;
        ctx.log(format!("解出 {}", base_name(&target)));
    }

    if zip_has_entry(&mut zip, "data/client.lzma") {
        // 1.13+ 需要跑 processor 生成 forge-client.jar
        let profile = read_install_profile(&mut zip)?;
        run_installer_processors(
            options,
            version_name,
            forge_version,
            &profile,
            &mut zip,
            &mc_json,
            ctx,
        )
        .await?;
    }

    let installer_json = read_installer_version_json(&mut zip)?;
    mc_json.merge(&installer_json)?;
    mc_json.extra.remove("_comment_");
    finish_version_json(&mut mc_json, version_name, true);
    write_json_safe(&path, &mc_json).await
}

/// `installNeoForge`。
pub async fn install_neoforge(
    options: &InstallOptions,
    version_name: &str,
    forge_version: &str,
    ctx: &TaskContext,
) -> Result<()> {
    let path = version_json_path(&options.root_path, version_name);
    let mut mc_json = read_version_json(&path).await?;
    let libraries = libraries_dir(&options.root_path);

    let (url, name) = neoforge_installer_url(forge_version);
    let installer = fetch_installer(
        options,
        url,
        format!("{name}-{forge_version}-installer.jar"),
        forge_version,
        ctx,
    )
    .await?;
    let mut zip = open_archive(installer)?;

    if zip_has_entry(&mut zip, "maven/") {
        extract_maven_entries(&mut zip, &libraries, ctx).await?;
    }

    // 这里没有 Forge 那条 `data/client.lzma` 判断（与原实现一致）
    let profile = read_install_profile(&mut zip)?;
    run_installer_processors(
        options,
        version_name,
        forge_version,
        &profile,
        &mut zip,
        &mc_json,
        ctx,
    )
    .await?;

    let installer_json = read_installer_version_json(&mut zip)?;
    mc_json.merge(&installer_json)?;
    mc_json.extra.remove("_comment_");
    mc_json.inherits_from = None;
    mc_json.id = Some(version_name.to_string());
    finish_version_json(&mut mc_json, version_name, true);
    write_json_safe(&path, &mc_json).await
}

// ---------------------------------------------------------------------------
// 任务
// ---------------------------------------------------------------------------

/// 安装 Fabric 的任务链（`loader_version = None` 时自动取最新稳定版）。
pub fn install_fabric_task(
    options: Arc<InstallOptions>,
    version_name: String,
    mc_version: String,
    loader_version: Option<String>,
) -> TaskChain<(), ()> {
    TaskChain::new(format!("安装 Fabric：{version_name}")).then(
        task_fn("安装 Fabric", move |ctx| {
            let options = options.clone();
            let version_name = version_name.clone();
            let mc_version = mc_version.clone();
            let loader_version = loader_version.clone();
            async move {
                let loader = match loader_version {
                    Some(loader) => loader,
                    None => {
                        ctx.log("查询最新的稳定 Fabric loader");
                        latest_fabric_loader(&options.source, &mc_version).await?
                    }
                };
                ctx.log(format!("使用 {mc_version}/{loader}"));
                install_fabric(&options, &version_name, &mc_version, &loader).await
            }
        }),
        "安装 Fabric",
    )
}

/// 安装 Forge 的任务链（`forge_version = None` 时自动取最新版）。
pub fn install_forge_task(
    options: Arc<InstallOptions>,
    version_name: String,
    mc_version: String,
    forge_version: Option<String>,
) -> TaskChain<(), ()> {
    TaskChain::new(format!("安装 Forge：{version_name}")).then(
        task_fn("安装 Forge", move |ctx| {
            let options = options.clone();
            let version_name = version_name.clone();
            let mc_version = mc_version.clone();
            let forge_version = forge_version.clone();
            async move {
                let version = match forge_version {
                    Some(version) => version,
                    None => {
                        ctx.log("查询最新的 Forge 版本");
                        latest_forge_version(&options.source, &mc_version).await?
                    }
                };
                ctx.log(format!("使用 Forge {version}"));
                install_forge(&options, &version_name, &version, &ctx).await
            }
        }),
        "安装 Forge",
    )
}

/// 安装 NeoForge 的任务链（`forge_version = None` 时自动取最新版）。
pub fn install_neoforge_task(
    options: Arc<InstallOptions>,
    version_name: String,
    mc_version: String,
    forge_version: Option<String>,
) -> TaskChain<(), ()> {
    TaskChain::new(format!("安装 NeoForge：{version_name}")).then(
        task_fn("安装 NeoForge", move |ctx| {
            let options = options.clone();
            let version_name = version_name.clone();
            let mc_version = mc_version.clone();
            let forge_version = forge_version.clone();
            async move {
                let version = match forge_version {
                    Some(version) => version,
                    None => {
                        ctx.log("查询最新的 NeoForge 版本");
                        latest_neoforge_version(&options.source, &mc_version).await?
                    }
                };
                ctx.log(format!("使用 NeoForge {version}"));
                install_neoforge(&options, &version_name, &version, &ctx).await
            }
        }),
        "安装 NeoForge",
    )
}

// ---------------------------------------------------------------------------
// 内部
// ---------------------------------------------------------------------------

/// 下载安装器到临时目录，返回其字节。
async fn fetch_installer(
    options: &InstallOptions,
    url: String,
    file_name: String,
    loader_label: &str,
    ctx: &TaskContext,
) -> Result<Vec<u8>> {
    let scratch = std::env::temp_dir().join(format!("dmcl-installer-{loader_label}"));
    let path = scratch.join(&file_name);
    download_planned(
        options,
        "安装器",
        vec![PlannedFile {
            url,
            save_to: path.clone(),
            checksum: None,
            size: None,
            name: file_name,
        }],
        ctx,
    )
    .await?;
    Ok(tokio::fs::read(&path).await?)
}

/// 安装包里 `maven/` 下的文件直接铺到 `libraries/`。
async fn extract_maven_entries(
    zip: &mut zip::ZipArchive<Cursor<Vec<u8>>>,
    libraries: &std::path::Path,
    ctx: &TaskContext,
) -> Result<()> {
    let count = zip.len();
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    for index in 0..count {
        let mut entry = zip.by_index(index)?;
        let name = entry.name().to_string();
        if !name.ends_with('/') && name.starts_with("maven/") {
            use std::io::Read;
            let mut buffer = Vec::new();
            entry.read_to_end(&mut buffer)?;
            files.push((name, buffer));
        }
    }
    for (name, buffer) in files {
        let target = libraries.join(&name["maven/".len()..]);
        write_safe(&target, &buffer).await?;
        ctx.log(format!("解出 {}", base_name(&name)));
    }
    Ok(())
}

fn read_install_profile(zip: &mut zip::ZipArchive<Cursor<Vec<u8>>>) -> Result<InstallProfile> {
    let bytes = zip_read_entry(zip, "install_profile.json")?;
    Ok(serde_json::from_slice(&bytes)?)
}

/// 从安装包里取出它自带的版本 JSON。
fn read_installer_version_json(zip: &mut zip::ZipArchive<Cursor<Vec<u8>>>) -> Result<VersionJson> {
    let mut json: VersionJson = if zip_has_entry(zip, "version.json") {
        serde_json::from_slice(&zip_read_entry(zip, "version.json")?)?
    } else {
        let profile = read_install_profile(zip)?;
        let entry = profile.file_path().unwrap_or("").to_string();
        let inner = zip_read_entry(zip, &entry)?;
        let mut inner_zip = open_archive(inner)?;
        serde_json::from_slice(&zip_read_entry(&mut inner_zip, "version.json")?)?
    };
    // 安装包里的地址是具体 url，归一到模板后写盘时才能再解析成官方地址
    json.templatize_urls();
    Ok(json)
}

/// 收尾：写盘前把 URL 模板解析回官方地址，并（Forge / NeoForge）记下客户端 jar 名。
fn finish_version_json(json: &mut VersionJson, version_name: &str, own_jar: bool) {
    json.resolve_urls(dmcl_mcmeta::Mirror::Official);
    json.inherits_from = None;
    json.id = Some(version_name.to_string());
    if own_jar {
        json.jar = Some(version_name.to_string());
    }
}

/// 检查版本 JSON 里是否有可用的 `mainClass`（安装完整性提示用）。
pub async fn version_is_launchable(options: &InstallOptions, version_name: &str) -> Result<bool> {
    let json = options.version_json(version_name).await?;
    if json.main_class.is_none() {
        return Err(anyhow!("{version_name} 的版本 JSON 里没有 mainClass"));
    }
    Ok(true)
}
