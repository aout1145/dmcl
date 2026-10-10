//! Forge / NeoForge 安装器的 processor 执行。
//!
//! 两个加载器里这段逻辑逐字节相同，因此只实现一次；调用方保持原有的顺序
//! （包括 Forge 独有的 `data/client.lzma` 判断）。
//!
//! processor 本身是外部 java 程序，所以直接用 [`CommandTask`] 跑：
//! 它的输出会逐行变成进度事件，失败时错误信息里带着输出结尾。

use anyhow::{anyhow, Context, Result};
use dmcl_download::check_file;
use dmcl_mcmeta::{classpath_separator, lib_name_to_path, libraries_dir, version_jar_path};
use dmcl_mcmeta::{InstallProfile, VersionJson};
use dmcl_task::{CommandTask, Task, TaskContext};

use crate::files::download_planned;
use crate::io::write_safe;
use crate::options::InstallOptions;
use crate::plan::{base_name, plan_libraries, PlannedFile};
use crate::zip::{main_class_of, open_archive, zip_read_entry};

/// 下载安装器声明的库，解出 `client.lzma`，然后依次执行客户端侧的 processor。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_installer_processors(
    options: &InstallOptions,
    version_name: &str,
    loader_label: &str,
    profile: &InstallProfile,
    zip: &mut zip::ZipArchive<std::io::Cursor<Vec<u8>>>,
    mc_json: &VersionJson,
    ctx: &TaskContext,
) -> Result<()> {
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
            sha1: artifact.sha1.clone(),
            size: artifact.size,
            name: base_name(&path),
        });
    }
    planned.extend(plan_libraries(options, mc_json));
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
        task.run(ctx.child(jar_coord))
            .await
            .with_context(|| format!("执行 {jar_coord} 失败"))?;

        // 校验产物
        if let Some(outputs) = &step.outputs {
            for (key, value) in outputs {
                let file = get_arg(key);
                let expected = get_arg(value);
                if !check_file(&file, Some(&expected)).await {
                    return Err(anyhow!("{file}: sha1 校验失败"));
                }
            }
        }
        ctx.status(format!("完成 {jar_coord}"));
    }

    Ok(())
}
