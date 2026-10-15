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

use anyhow::{anyhow, Context, Result};
use dmcl_download::{file_ok, Checksum};
use dmcl_mcmeta::layout::{libraries_dir, read_version_json, version_jar_path, version_json_path};
use dmcl_mcmeta::maven::lib_name_to_path;
use dmcl_mcmeta::os::classpath_separator;
use dmcl_mcmeta::vanilla::{Library, VersionJson};
use dmcl_task::{CommandTask, Task, TaskContext};

use crate::files::download_planned;
use crate::io::write_safe;
use crate::options::InstallOptions;
use crate::plan::{base_name, plan_libraries, PlannedFile};
use crate::zip::{main_class_of, open_archive, zip_read_entry};

/// 安装 Forge / NeoForge 时在步骤之间传递的中间值。
pub(crate) struct Installer {
    /// 安装器对应的加载器版本号。
    pub(crate) version: String,
    /// 安装器的下载地址。
    pub(crate) url: String,
    /// 安装器文件名。
    pub(crate) file_name: String,
    /// 安装器落地的临时路径。
    pub(crate) path: PathBuf,
}

/// 安装器临时路径：`<tmp>/dmcl-installer-<label>/<file_name>`。
pub(crate) fn installer_temp_path(label: &str, file_name: &str) -> PathBuf {
    std::env::temp_dir()
        .join(format!("dmcl-installer-{label}"))
        .join(file_name)
}

/// 下载安装器到临时路径。后续步骤按需重新打开，不把字节流长期攥在手里。
pub(crate) async fn download_installer(
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

/// 跑 processor 时需要的一项（两种 profile 的 `Processor` 类型不同，由调用方传进来）。
pub(crate) struct ProcessorSpec<'a> {
    pub jar: &'a str,
    pub classpath: &'a [String],
    pub args: &'a [String],
    pub outputs: Option<&'a BTreeMap<String, String>>,
}

/// 来自安装器档案的输入（两种 `InstallProfile` 类型不同，所以先拆成通用形状再传进来）。
pub(crate) struct ProfileInputs<'a> {
    /// 安装器档案声明的库。
    pub libraries: &'a [Library],
    /// 已经解析成 client 值的 `{占位符}` 数据。
    pub data: &'a BTreeMap<String, String>,
    /// 客户端侧的 processor（已过滤、已拆成视图）。
    pub processors: &'a [ProcessorSpec<'a>],
}

/// 下载安装器声明的库，解出 `client.lzma`，然后依次执行客户端侧的 processor。
pub(crate) async fn run_installer_processors(
    options: &InstallOptions,
    version_name: &str,
    loader_label: &str,
    inputs: ProfileInputs<'_>,
    zip: &mut zip::ZipArchive<Cursor<Vec<u8>>>,
    ctx: &TaskContext,
) -> Result<()> {
    let ProfileInputs {
        libraries: installer_libraries,
        data,
        processors,
    } = inputs;
    let mc_json = read_version_json(&version_json_path(&options.root_path, version_name)).await?;
    let libraries = libraries_dir(&options.root_path);

    // client.lzma 先解到临时目录（processor 的工作目录；下面解析参数要用到它的路径）
    let scratch = std::env::temp_dir().join(format!("dmcl-installer-{loader_label}"));
    let lzma_path = scratch.join("client.lzma");
    write_safe(&lzma_path, &zip_read_entry(zip, "data/client.lzma")?).await?;

    // 解析 `{占位符}`、lzma 路径、`[maven:坐标]` 以及外面的单引号
    let get_arg = |arg: &str| -> String {
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

    // 每个 processor 的实参（解析一次，下面复用）
    let resolved: Vec<Vec<String>> = processors
        .iter()
        .map(|spec| spec.args.iter().map(|arg| get_arg(arg)).collect())
        .collect();

    // 安装器声明的库 + 版本自己的库 + 可顶替的 DOWNLOAD_MOJMAPS，合成**一个**下载任务，
    // 让它们一起并发下；命中 mojmaps 的 processor 到时候直接跳过。
    let mut planned: Vec<PlannedFile> = Vec::new();
    for lib in installer_libraries {
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
    let mut mojmaps: Vec<bool> = vec![false; processors.len()];
    for index in 0..processors.len() {
        if let Some(file) = mojmaps_download(&mc_json, &resolved[index]) {
            planned.push(file);
            mojmaps[index] = true;
        }
    }
    download_planned(options, "安装器依赖库", planned, ctx).await?;

    // 需要哪个 java 由版本 JSON 决定，怎么找到 java 由调用者决定
    let java = options.java_for(mc_json.required_java_major()).await?;

    for (index, spec) in processors.iter().enumerate() {
        let jar_coord = spec.jar;

        // DOWNLOAD_MOJMAPS 已在上面那个下载任务里下好了，不用再跑 java
        if mojmaps[index] {
            ctx.log(format!("完成 {jar_coord}"));
            continue;
        }

        let jar_path = libraries.join(lib_name_to_path(jar_coord));

        // classpath：processor 自己的 jar 在最前，然后依次是它的 classpath
        let mut classpath = jar_path.to_string_lossy().into_owned();
        for entry in spec.classpath {
            classpath.push(classpath_separator());
            classpath.push_str(&libraries.join(lib_name_to_path(entry)).to_string_lossy());
        }

        let args = &resolved[index];

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
        if let Some(outputs) = spec.outputs {
            for (key, value) in outputs {
                let file = get_arg(key);
                let expected = Checksum::sha1(get_arg(value));
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
