//! Fabric / Forge / NeoForge 的版本查询与安装。
//!
//! 查询部分（[`fabric_loaders`]、[`forge_versions`] …）直接转调
//! [`dmcl_mcmeta::DownloadSource`]，把原实现里散落在下载模块里的接口收敛到这里；
//! "取最新版"的规则也与原实现逐条保持一致（见各函数文档）。
//!
//! 安装被拆成多个步骤组成的任务链：Fabric 两步，Forge / NeoForge 五步
//! （确定版本 → 下载安装器 → 解出安装器内的库 → 运行安装器 → 写版本元数据）。
//! 步骤之间用一个 `Installer` 值传参，不共享可变状态；每个阶段各自读一次磁盘。

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Result};
use dmcl_mcmeta::{
    forge_installer_url, lib_name_to_path, libraries_dir, neoforge_installer_url,
    read_version_json, version_json_path, DownloadSource, FabricLoaderEntry, ForgeVersionEntry,
    InstallProfile, VersionJson,
};
use dmcl_task::{task_fn_io, TaskChain, TaskContext};

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

/// 安装 Forge / NeoForge 时在步骤之间传递的中间值。
struct Installer {
    /// 安装器对应的加载器版本号。
    version: String,
    /// 安装器的下载地址。
    url: String,
    /// 安装器文件名。
    file_name: String,
    /// 安装器落地的临时路径。
    path: PathBuf,
}

/// 安装器临时路径：`<tmp>/dmcl-installer-<label>/<file_name>`。
fn installer_temp_path(label: &str, file_name: &str) -> PathBuf {
    std::env::temp_dir()
        .join(format!("dmcl-installer-{label}"))
        .join(file_name)
}

fn forge_installer(version: &str) -> Installer {
    let file_name = format!("forge-{version}-installer.jar");
    Installer {
        version: version.to_string(),
        url: forge_installer_url(version),
        path: installer_temp_path(version, &file_name),
        file_name,
    }
}

fn neoforge_installer(version: &str) -> Installer {
    let (url, name) = neoforge_installer_url(version);
    let file_name = format!("{name}-{version}-installer.jar");
    Installer {
        version: version.to_string(),
        url,
        path: installer_temp_path(version, &file_name),
        file_name,
    }
}

// --- Fabric ---

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

// --- Forge ---

/// `installForge`（薄封装：顺序跑各阶段）。
pub async fn install_forge(
    options: &InstallOptions,
    version_name: &str,
    forge_version: &str,
    ctx: &TaskContext,
) -> Result<()> {
    let installer = forge_installer(forge_version);
    download_installer(options, &installer, ctx).await?;
    extract_forge_libraries(options, forge_version, &installer.path, ctx).await?;
    run_forge_installer(options, version_name, forge_version, &installer.path, ctx).await?;
    write_installer_version_json(options, version_name, &installer.path).await
}

/// 解析 Forge 版本（`requested` 为 `None` 时取最新版）。
async fn resolve_forge_version(
    options: &InstallOptions,
    mc_version: &str,
    requested: Option<&str>,
    ctx: &TaskContext,
) -> Result<String> {
    let version = match requested {
        Some(version) => version.to_string(),
        None => {
            ctx.log("查询最新的 Forge 版本");
            latest_forge_version(&options.source, mc_version).await?
        }
    };
    ctx.log(format!("使用 Forge {version}"));
    Ok(version)
}

// --- NeoForge ---

/// `installNeoForge`（薄封装：顺序跑各阶段）。
pub async fn install_neoforge(
    options: &InstallOptions,
    version_name: &str,
    forge_version: &str,
    ctx: &TaskContext,
) -> Result<()> {
    let installer = neoforge_installer(forge_version);
    download_installer(options, &installer, ctx).await?;
    extract_neoforge_libraries(options, &installer.path, ctx).await?;
    run_neoforge_installer(options, version_name, forge_version, &installer.path, ctx).await?;
    write_installer_version_json(options, version_name, &installer.path).await
}

/// 解析 NeoForge 版本（`requested` 为 `None` 时取最新版）。
async fn resolve_neoforge_version(
    options: &InstallOptions,
    mc_version: &str,
    requested: Option<&str>,
    ctx: &TaskContext,
) -> Result<String> {
    let version = match requested {
        Some(version) => version.to_string(),
        None => {
            ctx.log("查询最新的 NeoForge 版本");
            latest_neoforge_version(&options.source, mc_version).await?
        }
    };
    ctx.log(format!("使用 NeoForge {version}"));
    Ok(version)
}

// --- 各阶段 ---

/// 下载安装器到临时路径。后续步骤按需重新打开，不把字节流长期攥在手里。
async fn download_installer(
    options: &InstallOptions,
    installer: &Installer,
    ctx: &TaskContext,
) -> Result<()> {
    download_planned(
        options,
        "安装器",
        vec![PlannedFile {
            url: installer.url.clone(),
            save_to: installer.path.clone(),
            checksum: None,
            size: None,
            name: installer.file_name.clone(),
        }],
        ctx,
    )
    .await
    .map(|_| ())
}

/// Forge：解出安装器里 `maven/` 下的文件；老版把单个 jar 直接铺进 `libraries/`。
async fn extract_forge_libraries(
    options: &InstallOptions,
    forge_version: &str,
    installer: &Path,
    ctx: &TaskContext,
) -> Result<()> {
    let libraries = libraries_dir(&options.root_path);
    let bytes = tokio::fs::read(installer).await?;
    let mut zip = open_archive(bytes)?;

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
    Ok(())
}

/// NeoForge：解出安装器里 `maven/` 下的文件。
async fn extract_neoforge_libraries(
    options: &InstallOptions,
    installer: &Path,
    ctx: &TaskContext,
) -> Result<()> {
    let libraries = libraries_dir(&options.root_path);
    let bytes = tokio::fs::read(installer).await?;
    let mut zip = open_archive(bytes)?;
    if zip_has_entry(&mut zip, "maven/") {
        extract_maven_entries(&mut zip, &libraries, ctx).await?;
    }
    Ok(())
}

/// Forge：1.13+ 跑安装器 processor 生成 forge-client.jar。
async fn run_forge_installer(
    options: &InstallOptions,
    version_name: &str,
    forge_version: &str,
    installer: &Path,
    ctx: &TaskContext,
) -> Result<()> {
    let bytes = tokio::fs::read(installer).await?;
    let mut zip = open_archive(bytes)?;
    if zip_has_entry(&mut zip, "data/client.lzma") {
        run_processors(options, version_name, forge_version, &mut zip, ctx).await?;
    }
    Ok(())
}

/// NeoForge：总是跑安装器 processor（没有 Forge 那条 `data/client.lzma` 判断）。
async fn run_neoforge_installer(
    options: &InstallOptions,
    version_name: &str,
    forge_version: &str,
    installer: &Path,
    ctx: &TaskContext,
) -> Result<()> {
    let bytes = tokio::fs::read(installer).await?;
    let mut zip = open_archive(bytes)?;
    run_processors(options, version_name, forge_version, &mut zip, ctx).await
}

/// 打开安装器、读 profile 与本地版本 JSON，跑 processor。
async fn run_processors(
    options: &InstallOptions,
    version_name: &str,
    loader_label: &str,
    zip: &mut zip::ZipArchive<Cursor<Vec<u8>>>,
    ctx: &TaskContext,
) -> Result<()> {
    let path = version_json_path(&options.root_path, version_name);
    let mc_json = read_version_json(&path).await?;
    let profile = read_install_profile(zip)?;
    run_installer_processors(
        options,
        version_name,
        loader_label,
        &profile,
        zip,
        &mc_json,
        ctx,
    )
    .await
}

/// 把安装器自带的版本 JSON 合并进本地版本 JSON 并写盘（Forge / NeoForge 相同）。
async fn write_installer_version_json(
    options: &InstallOptions,
    version_name: &str,
    installer: &Path,
) -> Result<()> {
    let path = version_json_path(&options.root_path, version_name);
    let mut mc_json = read_version_json(&path).await?;

    let bytes = tokio::fs::read(installer).await?;
    let mut zip = open_archive(bytes)?;
    let installer_json = read_installer_version_json(&mut zip)?;
    mc_json.merge(&installer_json)?;
    mc_json.extra.remove("_comment_");
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

/// 安装 Forge 的任务链（`forge_version = None` 时自动取最新版）。
pub fn install_forge_task(
    options: Arc<InstallOptions>,
    version_name: String,
    mc_version: String,
    forge_version: Option<String>,
) -> TaskChain<(), ()> {
    TaskChain::new(format!("安装 Forge：{version_name}"))
        .then(
            task_fn_io("确定 Forge 版本", {
                let options = options.clone();
                let mc_version = mc_version.clone();
                move |_: (), ctx: TaskContext| {
                    let options = options.clone();
                    let mc_version = mc_version.clone();
                    let forge_version = forge_version.clone();
                    async move {
                        let version = resolve_forge_version(
                            &options,
                            &mc_version,
                            forge_version.as_deref(),
                            &ctx,
                        )
                        .await?;
                        Ok(forge_installer(&version))
                    }
                }
            }),
            "确定 Forge 版本",
        )
        .then(
            task_fn_io("下载 Forge 安装器", {
                let options = options.clone();
                move |installer: Installer, ctx: TaskContext| {
                    let options = options.clone();
                    async move {
                        download_installer(&options, &installer, &ctx).await?;
                        Ok(installer)
                    }
                }
            }),
            "下载 Forge 安装器",
        )
        .then(
            task_fn_io("解出安装器内的库", {
                let options = options.clone();
                move |installer: Installer, ctx: TaskContext| {
                    let options = options.clone();
                    async move {
                        extract_forge_libraries(
                            &options,
                            &installer.version,
                            &installer.path,
                            &ctx,
                        )
                        .await?;
                        Ok(installer)
                    }
                }
            }),
            "解出安装器内的库",
        )
        .then(
            task_fn_io("运行安装器", {
                let options = options.clone();
                let version_name = version_name.clone();
                move |installer: Installer, ctx: TaskContext| {
                    let options = options.clone();
                    let version_name = version_name.clone();
                    async move {
                        run_forge_installer(
                            &options,
                            &version_name,
                            &installer.version,
                            &installer.path,
                            &ctx,
                        )
                        .await?;
                        Ok(installer)
                    }
                }
            }),
            "运行安装器",
        )
        .then(
            task_fn_io("写入版本元数据", {
                let options = options.clone();
                let version_name = version_name.clone();
                move |installer: Installer, _ctx: TaskContext| {
                    let options = options.clone();
                    let version_name = version_name.clone();
                    async move {
                        write_installer_version_json(&options, &version_name, &installer.path).await
                    }
                }
            }),
            "写入版本元数据",
        )
}

/// 安装 NeoForge 的任务链（`forge_version = None` 时自动取最新版）。
pub fn install_neoforge_task(
    options: Arc<InstallOptions>,
    version_name: String,
    mc_version: String,
    forge_version: Option<String>,
) -> TaskChain<(), ()> {
    TaskChain::new(format!("安装 NeoForge：{version_name}"))
        .then(
            task_fn_io("确定 NeoForge 版本", {
                let options = options.clone();
                let mc_version = mc_version.clone();
                move |_: (), ctx: TaskContext| {
                    let options = options.clone();
                    let mc_version = mc_version.clone();
                    let forge_version = forge_version.clone();
                    async move {
                        let version = resolve_neoforge_version(
                            &options,
                            &mc_version,
                            forge_version.as_deref(),
                            &ctx,
                        )
                        .await?;
                        Ok(neoforge_installer(&version))
                    }
                }
            }),
            "确定 NeoForge 版本",
        )
        .then(
            task_fn_io("下载 NeoForge 安装器", {
                let options = options.clone();
                move |installer: Installer, ctx: TaskContext| {
                    let options = options.clone();
                    async move {
                        download_installer(&options, &installer, &ctx).await?;
                        Ok(installer)
                    }
                }
            }),
            "下载 NeoForge 安装器",
        )
        .then(
            task_fn_io("解出安装器内的库", {
                let options = options.clone();
                move |installer: Installer, ctx: TaskContext| {
                    let options = options.clone();
                    async move {
                        extract_neoforge_libraries(&options, &installer.path, &ctx).await?;
                        Ok(installer)
                    }
                }
            }),
            "解出安装器内的库",
        )
        .then(
            task_fn_io("运行安装器", {
                let options = options.clone();
                let version_name = version_name.clone();
                move |installer: Installer, ctx: TaskContext| {
                    let options = options.clone();
                    let version_name = version_name.clone();
                    async move {
                        run_neoforge_installer(
                            &options,
                            &version_name,
                            &installer.version,
                            &installer.path,
                            &ctx,
                        )
                        .await?;
                        Ok(installer)
                    }
                }
            }),
            "运行安装器",
        )
        .then(
            task_fn_io("写入版本元数据", {
                let options = options.clone();
                let version_name = version_name.clone();
                move |installer: Installer, _ctx: TaskContext| {
                    let options = options.clone();
                    let version_name = version_name.clone();
                    async move {
                        write_installer_version_json(&options, &version_name, &installer.path).await
                    }
                }
            }),
            "写入版本元数据",
        )
}

// ---------------------------------------------------------------------------
// 内部
// ---------------------------------------------------------------------------

/// 安装包里 `maven/` 下的文件直接铺到 `libraries/`。
async fn extract_maven_entries(
    zip: &mut zip::ZipArchive<Cursor<Vec<u8>>>,
    libraries: &Path,
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

#[cfg(test)]
mod tests {
    use super::*;
    use dmcl_mcmeta::Mirror;

    fn options() -> Arc<InstallOptions> {
        Arc::new(InstallOptions::new(
            "/tmp/.minecraft",
            DownloadSource::new(Mirror::Official),
        ))
    }

    #[test]
    fn forge_install_is_five_named_steps() {
        let chain = install_forge_task(
            options(),
            "1.20.1".into(),
            "1.20.1".into(),
            Some("47.2.0".into()),
        );
        let names: Vec<_> = chain.step_names().collect();
        assert_eq!(
            names,
            [
                "确定 Forge 版本",
                "下载 Forge 安装器",
                "解出安装器内的库",
                "运行安装器",
                "写入版本元数据",
            ]
        );
    }

    #[test]
    fn neoforge_install_is_five_named_steps() {
        let chain = install_neoforge_task(
            options(),
            "1.20.1".into(),
            "1.20.1".into(),
            Some("47.1.0".into()),
        );
        assert_eq!(chain.step_names().count(), 5);
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
