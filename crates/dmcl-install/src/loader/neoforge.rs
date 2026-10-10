//! NeoForge 的安装（查询见 [`dmcl_mcmeta`]）。

use std::io::Cursor;
use std::path::Path;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use dmcl_download::{check_file, Checksum};
use dmcl_mcmeta::{
    classpath_separator, latest_neoforge_version, lib_name_to_path, libraries_dir,
    neoforge_installer_url, read_version_json, version_jar_path, version_json_path,
};
use dmcl_task::{task_fn_io, CommandTask, Task, TaskChain, TaskContext};

use crate::files::download_planned;
use crate::io::write_safe;
use crate::loader::common::{
    download_installer, extract_maven_entries, installer_temp_path, read_install_profile,
    write_installer_version_json, Installer,
};
use crate::options::InstallOptions;
use crate::plan::{base_name, plan_libraries, PlannedFile};
use crate::zip::{main_class_of, open_archive, zip_has_entry, zip_read_entry};

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

/// 解出安装器里 `maven/` 下的文件。
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

/// 跑安装器 processor（NeoForge 没有 Forge 那条 `data/client.lzma` 判断）。
async fn run_neoforge_installer(
    options: &InstallOptions,
    version_name: &str,
    forge_version: &str,
    installer: &Path,
    ctx: &TaskContext,
) -> Result<()> {
    let bytes = tokio::fs::read(installer).await?;
    let mut zip = open_archive(bytes)?;
    run_installer_processors(options, version_name, forge_version, &mut zip, ctx).await
}

/// 下载安装器声明的库，解出 `client.lzma`，然后依次执行客户端侧的 processor。
///
/// NeoForge 自己的一份（与 Forge 今天相同、以后可能不同）。
async fn run_installer_processors(
    options: &InstallOptions,
    version_name: &str,
    loader_label: &str,
    zip: &mut zip::ZipArchive<Cursor<Vec<u8>>>,
    ctx: &TaskContext,
) -> Result<()> {
    let mc_json = read_version_json(&version_json_path(&options.root_path, version_name)).await?;
    let profile = read_install_profile(zip)?;
    let libraries = libraries_dir(&options.root_path);

    // 1) 安装器声明的库 + 版本自己的库
    let mut planned: Vec<PlannedFile> = Vec::new();
    for lib in &profile.libraries {
        let Some(artifact) = lib.downloads.as_ref().and_then(|d| d.artifact.as_ref()) else {
            continue;
        };
        let (Some(url), Some(path)) = (artifact.url.clone(), artifact.path.clone()) else {
            continue;
        };
        planned.push(PlannedFile {
            url,
            save_to: libraries.join(&path),
            checksum: artifact.sha1.clone().map(Checksum::sha1),
            size: artifact.size,
            name: base_name(&path),
        });
    }
    planned.extend(plan_libraries(&options.plan(), &mc_json));
    download_planned(options, "安装器依赖库", planned, ctx).await?;

    // 2) client.lzma 解到一个临时目录（processor 的工作目录）
    let scratch = std::env::temp_dir().join(format!("dmcl-installer-{loader_label}"));
    let lzma_path = scratch.join("client.lzma");
    write_safe(&lzma_path, &zip_read_entry(zip, "data/client.lzma")?).await?;

    // 3) 需要哪个 java 由版本 JSON 决定，怎么找到 java 由调用者决定
    let java = options.java_for(mc_json.required_java_major()).await?;

    // `getArg`：解析 `{占位符}`、lzma 路径、`[maven:坐标]` 以及外面的单引号
    let get_arg = |arg: &str| -> String {
        let mut arg = arg.to_string();
        if arg.len() >= 2 && arg.starts_with('{') && arg.ends_with('}') {
            let inner = &arg[1..arg.len() - 1];
            arg = match inner {
                "MINECRAFT_JAR" => version_jar_path(&options.root_path, version_name)
                    .to_string_lossy()
                    .into_owned(),
                "SIDE" => "client".to_string(),
                key => profile
                    .data
                    .get(key)
                    .and_then(|entry| entry.client())
                    .unwrap_or("")
                    .to_string(),
            };
        }
        if arg == "/data/client.lzma" {
            arg = lzma_path.to_string_lossy().into_owned();
        }
        if arg.len() >= 2 && arg.starts_with('[') && arg.ends_with(']') {
            let inner = &arg[1..arg.len() - 1];
            arg = libraries
                .join(lib_name_to_path(inner))
                .to_string_lossy()
                .into_owned();
        }
        if arg.len() >= 2 && arg.starts_with('\'') && arg.ends_with('\'') {
            arg = arg[1..arg.len() - 1].to_string();
        }
        arg
    };

    for step in &profile.processors {
        if !step.is_client_side() {
            continue;
        }
        let jar_coord = step.jar.as_str();
        let jar_path = libraries.join(lib_name_to_path(jar_coord));

        // classpath：processor 自己的 jar 在最前，然后依次是它的 classpath
        let mut classpath = jar_path.to_string_lossy().into_owned();
        for entry in &step.classpath {
            classpath.push(classpath_separator());
            classpath.push_str(&libraries.join(lib_name_to_path(entry)).to_string_lossy());
        }

        let args: Vec<String> = step.args.iter().map(|arg| get_arg(arg)).collect();

        // 入口类从 processor jar 的 MANIFEST 里读
        let bytes = tokio::fs::read(&jar_path)
            .await
            .with_context(|| format!("无法读取 processor jar {}", jar_path.display()))?;
        let mut archive = open_archive(bytes)?;
        let main_class = main_class_of(&mut archive)?;
        if main_class.is_empty() {
            return Err(anyhow!("{jar_coord} 的 MANIFEST 里没有 Main-Class"));
        }

        let task = CommandTask::new(java.as_os_str().to_os_string())
            .named(jar_coord.to_string())
            .arg("-cp")
            .arg(&classpath)
            .arg(&main_class)
            .args(args.iter().cloned())
            .current_dir(&scratch);
        task.run((), ctx.child(jar_coord))
            .await
            .with_context(|| format!("执行 {jar_coord} 失败"))?;

        // 校验产物（安装器档案里的 `outputs` 给的是 sha1 摘要）
        if let Some(outputs) = &step.outputs {
            for (key, value) in outputs {
                let file = get_arg(key);
                let expected = Checksum::sha1(get_arg(value));
                if !check_file(&file, Some(&expected)).await {
                    return Err(anyhow!("{file}: sha1 校验失败"));
                }
            }
        }
        ctx.log(format!("完成 {jar_coord}"));
    }

    Ok(())
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
    fn neoforge_install_is_five_named_steps() {
        let chain = install_neoforge_task(
            options(),
            "1.20.1".into(),
            "1.20.1".into(),
            Some("47.1.0".into()),
        );
        let names: Vec<_> = chain.step_names().collect();
        assert_eq!(
            names,
            [
                "确定 NeoForge 版本",
                "下载 NeoForge 安装器",
                "解出安装器内的库",
                "运行安装器",
                "写入版本元数据",
            ]
        );
    }
}
