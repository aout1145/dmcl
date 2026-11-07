//! Forge 与 NeoForge 安装共用的内部工具。
//!
//! 这两种加载器都是「下载安装器 jar → 跑 processor」，这里放它们共同用到的东西；
//! 三种加载器都用的收尾（版本 JSON 落盘）在 [`crate::loader::common`]。
//!
//! 两种加载器的 `InstallProfile` 是**各自独立的类型**，所以这里不直接收 profile，
//! 而是接收已经拆出来的通用参数（库列表、client data、processor 视图）。

use std::collections::BTreeMap;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use dmcl_download::{file_ok, Checksum, DownloadContext};
use dmcl_game::{base_name, download_planned, plan_libraries, PlannedFile, ResolvedVersion};
use dmcl_mcmeta::layout::{libraries_dir, version_jar_path};
use dmcl_mcmeta::maven::lib_name_to_path;
use dmcl_mcmeta::os::classpath_separator;
use dmcl_mcmeta::vanilla::{Library, VersionJson};
use dmcl_task::{task_fn, CommandTask, StepId, Task, TaskContext};

use crate::archive::{main_class_of, open_archive};
use crate::io::write_safe;
use crate::options::InstallOptions;

/// 安装 Forge / NeoForge 时在步骤之间传递的中间值。
#[derive(Clone)]
pub(crate) struct Installer {
    /// 安装器的下载地址。
    pub(crate) url: String,
    /// 安装器文件名。
    pub(crate) file_name: String,
    /// 安装器落地的临时路径。
    pub(crate) path: PathBuf,
}

/// 从安装器里解析出来的、后续阶段要用的内容（自有数据，不借用 zip）。
#[derive(Clone)]
pub(crate) struct InstallerPlan {
    pub(crate) installer: Installer,
    /// 安装器自带的版本 JSON（地址已模板化）。
    pub(crate) installer_json: VersionJson,
    /// 安装器档案声明的库。
    pub(crate) libraries: Vec<Library>,
    /// 已解析成 client 值的 `{占位符}` 数据。
    pub(crate) data: BTreeMap<String, String>,
    /// 客户端侧的 processor。
    pub(crate) processors: Vec<ProcessorPlan>,
    /// 安装器里的 `data/client.lzma`；老式安装器没有。
    pub(crate) client_lzma: Option<Vec<u8>>,
}

/// 一个客户端侧 processor 的静态描述。
#[derive(Clone)]
pub(crate) struct ProcessorPlan {
    pub(crate) jar: String,
    pub(crate) classpath: Vec<String>,
    pub(crate) args: Vec<String>,
    pub(crate) outputs: Option<BTreeMap<String, String>>,
}

/// 下载安装器依赖后得到的运行环境。
pub(crate) struct InstallerDeps {
    /// 需要跑 processor 时选好的 Java；全部是 mojmaps 下载时为空。
    pub(crate) java: Option<PathBuf>,
    /// 每个 processor 是否已被等价下载接管（命中后不用跑 java）。
    pub(crate) mojmaps: Vec<bool>,
    /// 解析过 `{占位符}` 的 processor 实参。
    pub(crate) args: Vec<Vec<String>>,
}

/// 安装器临时路径：`<tmp>/dmcl-installer-<label>/<file_name>`。
pub(crate) fn installer_temp_path(label: &str, file_name: &str) -> PathBuf {
    installer_scratch(label).join(file_name)
}

/// processor 的临时工作目录：`<tmp>/dmcl-installer-<label>`。
pub(crate) fn installer_scratch(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!("dmcl-installer-{label}"))
}

/// 下载安装器到临时路径。后续步骤按需重新打开，不把字节流长期攥在手里。
pub(crate) async fn download_installer(
    options: &InstallOptions,
    downloads: &Arc<DownloadContext>,
    installer: &Installer,
    ctx: &TaskContext,
) -> Result<()> {
    download_planned(
        &options.game,
        downloads,
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
}

/// 安装包里 `maven/` 下的文件直接铺到 `libraries/`。
pub(crate) async fn extract_maven_entries(
    zip: &mut zip::ZipArchive<Cursor<Vec<u8>>>,
    libraries: &Path,
    ctx: &TaskContext,
) -> Result<()> {
    let count = zip.len();
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    for index in 0..count {
        let mut entry = zip.by_index(index)?;
        let name = entry.name()?.to_string();
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

/// 读取安装器（或版本）要求的 Java 大版本，选出（必要时下载）processor 要用的 Java。
pub(crate) async fn select_installer_java(
    options: &InstallOptions,
    json: &VersionJson,
    downloads: &Arc<DownloadContext>,
    ctx: &TaskContext,
) -> Result<PathBuf> {
    let preferred = json
        .java_version
        .as_ref()
        .and_then(|java| java.component.as_deref())
        .filter(|component| !component.is_empty());
    options
        .java_for_or_install(json.required_java_major(), preferred, downloads, ctx)
        .await
}

/// 把一个 processor 参数里的占位符、lzma 路径与 maven 坐标解析成实际值。
fn resolve_arg(
    arg: &str,
    options: &InstallOptions,
    version_name: &str,
    data: &BTreeMap<String, String>,
    lzma_path: &Path,
) -> String {
    let mut arg = arg.to_string();
    if arg.len() >= 2 && arg.starts_with('{') && arg.ends_with('}') {
        let inner = &arg[1..arg.len() - 1];
        arg = match inner {
            "MINECRAFT_JAR" => version_jar_path(&options.root_path, version_name)
                .to_string_lossy()
                .into_owned(),
            "SIDE" => "client".to_string(),
            key => data.get(key).cloned().unwrap_or_default(),
        };
    }
    if arg == "/data/client.lzma" {
        arg = lzma_path.to_string_lossy().into_owned();
    }
    if arg.len() >= 2 && arg.starts_with('[') && arg.ends_with(']') {
        let inner = &arg[1..arg.len() - 1];
        arg = libraries_dir(&options.root_path)
            .join(lib_name_to_path(inner))
            .to_string_lossy()
            .into_owned();
    }
    if arg.len() >= 2 && arg.starts_with('\'') && arg.ends_with('\'') {
        arg = arg[1..arg.len() - 1].to_string();
    }
    arg
}

/// 下载安装器声明的库、版本自己的库与可顶替的 `DOWNLOAD_MOJMAPS`，必要时选好 Java。
///
/// 命中的 mojmaps 会并进同一个下载任务，后面跑 processor 时直接跳过。
/// Java 选择作为「下载游戏文件」Step 下的动态任务创建，JRE 下载也因此归在同一个 Step。
#[expect(clippy::too_many_arguments)]
pub(crate) async fn download_installer_dependencies(
    options: &InstallOptions,
    downloads: &Arc<DownloadContext>,
    version_name: &str,
    loader_label: &str,
    download_step: StepId,
    previous: &ResolvedVersion,
    plan: &InstallerPlan,
    ctx: &TaskContext,
) -> Result<InstallerDeps> {
    // 老式安装器没有 processor：库与 mojmaps 都留给「下载游戏文件」Step 统一处理。
    if plan.processors.is_empty() {
        return Ok(InstallerDeps {
            java: None,
            mojmaps: Vec::new(),
            args: Vec::new(),
        });
    }

    let lzma_path = installer_scratch(loader_label).join("client.lzma");
    let args: Vec<Vec<String>> = plan
        .processors
        .iter()
        .map(|processor| {
            processor
                .args
                .iter()
                .map(|arg| resolve_arg(arg, options, version_name, &plan.data, &lzma_path))
                .collect()
        })
        .collect();

    let libraries = libraries_dir(&options.root_path);
    let mut planned: Vec<PlannedFile> = Vec::new();
    for lib in &plan.libraries {
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
    planned.extend(plan_libraries(&previous.reference, &previous.json));

    let mut mojmaps: Vec<bool> = vec![false; plan.processors.len()];
    for (index, resolved) in args.iter().enumerate() {
        if let Some(file) = mojmaps_download(&previous.json, resolved) {
            planned.push(file);
            mojmaps[index] = true;
        }
    }

    let needs_java = mojmaps.iter().any(|replaced| !replaced);
    let java = if needs_java {
        let java_options = options.clone();
        let java_downloads = downloads.clone();
        let json = previous.json.clone();
        let handle = ctx.add_task(
            download_step,
            "选择 Java",
            (),
            task_fn(move |_: (), ctx| {
                let options = java_options.clone();
                let downloads = java_downloads.clone();
                let json = json.clone();
                async move { select_installer_java(&options, &json, &downloads, &ctx).await }
            }),
        )?;
        Some(handle)
    } else {
        None
    };

    download_planned(&options.game, downloads, "安装器依赖库", planned, ctx).await?;

    let java = match java {
        Some(handle) => Some((*handle.wait().await?).clone()),
        None => None,
    };
    Ok(InstallerDeps {
        java,
        mojmaps,
        args,
    })
}

/// 依次执行客户端侧的 processor。
///
/// 依赖已经由 [`download_installer_dependencies`] 提前下载好：这里只跑 Java，
/// 不再自己下载库或 mojmaps。
pub(crate) async fn run_installer_processors(
    options: &InstallOptions,
    version_name: &str,
    loader_label: &str,
    plan: &InstallerPlan,
    deps: &InstallerDeps,
    ctx: &TaskContext,
) -> Result<()> {
    if plan.processors.is_empty() {
        return Ok(());
    }

    let scratch = installer_scratch(loader_label);
    tokio::fs::create_dir_all(&scratch)
        .await
        .with_context(|| format!("无法创建 {}", scratch.display()))?;
    let lzma_path = scratch.join("client.lzma");
    if let Some(bytes) = &plan.client_lzma {
        write_safe(&lzma_path, bytes).await?;
    }

    let libraries = libraries_dir(&options.root_path);
    for (index, spec) in plan.processors.iter().enumerate() {
        let jar_coord = &spec.jar;

        // DOWNLOAD_MOJMAPS 已由依赖下载接管，不用再跑 java
        if deps.mojmaps[index] {
            ctx.log(format!("完成 {jar_coord}"));
            continue;
        }

        let java = deps
            .java
            .as_deref()
            .ok_or_else(|| anyhow!("安装器需要 Java，但没有选到可用的 Java"))?;
        let jar_path = libraries.join(lib_name_to_path(jar_coord));

        // classpath：processor 自己的 jar 在最前，然后依次是它的 classpath
        let mut classpath = jar_path.to_string_lossy().into_owned();
        for entry in &spec.classpath {
            classpath.push(classpath_separator());
            classpath.push_str(&libraries.join(lib_name_to_path(entry)).to_string_lossy());
        }

        let args = &deps.args[index];

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
            .arg("-cp")
            .arg(&classpath)
            .arg(&main_class)
            .args(args.iter().cloned())
            .current_dir(&scratch);
        task.run((), ctx.clone())
            .await
            .with_context(|| format!("执行 {jar_coord} 失败"))?;

        // 校验产物（安装器档案里的 `outputs` 给的是 sha1 摘要）
        if let Some(outputs) = &spec.outputs {
            for (key, value) in outputs {
                let file = resolve_arg(key, options, version_name, &plan.data, &lzma_path);
                let expected = Checksum::sha1(resolve_arg(
                    value,
                    options,
                    version_name,
                    &plan.data,
                    &lzma_path,
                ));
                if !file_ok(&file, Some(&expected), None).await {
                    return Err(anyhow!("{file}: sha1 校验失败"));
                }
            }
        }
        ctx.log(format!("完成 {jar_coord}"));
    }

    Ok(())
}

/// 如果这个 processor 是 installertools 的 `DOWNLOAD_MOJMAPS`（把 Mojang 官方混淆映射
/// 原样下到 `--output`），返回该下载项 —— 调用方把它并进自己的下载任务（走镜像 / 重试 /
/// 并发进度），命中它的 processor 就不用再跑 java 了。
///
/// `args` 是**已经解析过 `{占位符}`** 的参数。版本 JSON 没有对应 side 的
/// `*_mappings`（老版本）时返回 `None`，仍交给安装器处理。
pub(crate) fn mojmaps_download(json: &VersionJson, args: &[String]) -> Option<PlannedFile> {
    if flag_value(args, "--task")? != "DOWNLOAD_MOJMAPS" {
        return None;
    }
    let output = flag_value(args, "--output")?;
    let downloads = json.downloads.as_ref()?;
    let artifact = match flag_value(args, "--side").unwrap_or("client") {
        "server" => downloads.server_mappings.as_ref()?,
        _ => downloads.client_mappings.as_ref()?,
    };
    let url = artifact.url.clone()?;
    Some(PlannedFile {
        url,
        save_to: PathBuf::from(output),
        checksum: artifact.sha1.clone().map(Checksum::sha1),
        size: artifact.size,
        name: base_name(output).to_string(),
    })
}

/// 取 `--flag value` 里的 value。
fn flag_value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    let index = args.iter().position(|arg| arg == flag)?;
    args.get(index + 1).map(String::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version_with_mappings() -> VersionJson {
        serde_json::from_str(
            r#"{
                "downloads": {
                    "client_mappings": {
                        "url": "https://piston-data.mojang.com/x/client.txt",
                        "sha1": "abc", "size": 123
                    },
                    "server_mappings": {
                        "url": "https://piston-data.mojang.com/x/server.txt",
                        "sha1": "def", "size": 456
                    }
                }
            }"#,
        )
        .unwrap()
    }

    fn args(task: &str, side: &str) -> Vec<String> {
        vec![
            "--task".into(),
            task.into(),
            "--side".into(),
            side.into(),
            "--output".into(),
            "/out/mappings.txt".into(),
        ]
    }

    #[test]
    fn mojmaps_task_becomes_a_planned_download() {
        let json = version_with_mappings();
        let planned =
            mojmaps_download(&json, &args("DOWNLOAD_MOJMAPS", "client")).expect("应当识别");
        assert_eq!(planned.url, "https://piston-data.mojang.com/x/client.txt");
        assert_eq!(planned.save_to, PathBuf::from("/out/mappings.txt"));
        assert_eq!(planned.size, Some(123));
        assert_eq!(planned.checksum.as_ref().map(Checksum::hex), Some("abc"));

        // server 侧取 server_mappings
        let server = mojmaps_download(&json, &args("DOWNLOAD_MOJMAPS", "server")).unwrap();
        assert_eq!(server.url, "https://piston-data.mojang.com/x/server.txt");
    }

    #[test]
    fn other_tasks_are_left_to_the_installer() {
        let json = version_with_mappings();
        // 不是 DOWNLOAD_MOJMAPS 就不接管
        assert!(mojmaps_download(&json, &args("MERGE_MAPPING", "client")).is_none());
        // 版本 JSON 没有映射信息时也不接管（仍交给安装器）
        let bare: VersionJson = serde_json::from_str("{}").unwrap();
        assert!(mojmaps_download(&bare, &args("DOWNLOAD_MOJMAPS", "client")).is_none());
    }
}
