//! OptiFine 的安装（查询见 [`dmcl_mcmeta::optifine`]）。

use std::io::Cursor;
use std::sync::Arc;

use anyhow::{Context, Result};
use dmcl_download::DownloadContext;
use dmcl_game::{base_name, download_planned, PlannedFile};
use dmcl_mcmeta::json::JsonMap;
use dmcl_mcmeta::layout::libraries_dir;
use dmcl_mcmeta::maven::lib_name_to_path;
use dmcl_mcmeta::vanilla::{Library, VersionJsonPatch};
use dmcl_task::{task_fn, Handle, StepId, TaskContext, TaskGraphBuilder};

use crate::archive::{has_entry, open_archive, read_entry};
use crate::io::write_safe;
use crate::options::InstallOptions;

/// OptiFine 的库坐标前缀。
const OPTIFINE_LIBRARY: &str = "optifine:OptiFine";
/// launchwrapper 的库坐标前缀。
const LAUNCHWRAPPER_LIBRARY: &str = "net.minecraft:launchwrapper";

/// 下载 OptiFine 得到的产物：生成 patch 只需要这里的版本号。
#[derive(Debug, Clone)]
pub struct OptifineFiles {
    /// `optifine:OptiFine:<version>` 里的版本部分。
    pub library_version: String,
    /// 从 jar 里解出的 launchwrapper 版本；有 Forge 时为 `None`。
    pub launchwrapper: Option<String>,
}

/// 往安装图里追加 OptiFine：先并行下载 jar，再生成 patch。
///
/// 「下载 OptiFine」只依赖版本列表与共享的 `libraries/`，因此和原版元数据、版本 jar、
/// 资源、Forge 安装同时起跑。
pub(crate) fn add_optifine_install(
    builder: &mut TaskGraphBuilder,
    step: StepId,
    options: Arc<InstallOptions>,
    downloads: Arc<DownloadContext>,
    mc_version: String,
    optifine_version: String,
    has_forge: bool,
) -> Handle<VersionJsonPatch> {
    let files = builder.add(
        step,
        "下载 OptiFine",
        (),
        task_fn({
            let options = options.clone();
            let downloads = downloads.clone();
            let mc_version = mc_version.clone();
            let optifine_version = optifine_version.clone();
            move |_: (), ctx: TaskContext| {
                let options = options.clone();
                let downloads = downloads.clone();
                let mc_version = mc_version.clone();
                let optifine_version = optifine_version.clone();
                async move {
                    download_optifine(
                        &options,
                        &downloads,
                        &mc_version,
                        &optifine_version,
                        has_forge,
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
        files,
        task_fn(
            move |files: Arc<OptifineFiles>, ctx: TaskContext| async move {
                let patch = install_optifine(&files, has_forge);
                ctx.log(format!("已生成 OptiFine {} patch", patch.version));
                Ok(patch)
            },
        ),
    )
}

/// 下载 OptiFine jar，并在没有 Forge 时解出它自带的 launchwrapper。
///
/// 只依赖 OptiFine 的版本列表与共享的 `libraries/` 目录，不碰版本 JSON，因此可以和
/// 原版元数据、版本 jar、资源、Forge 安装等步骤并行。`has_forge` 决定是否沿用 Forge
/// 提供的 launchwrapper。
pub async fn download_optifine(
    options: &InstallOptions,
    downloads: &Arc<DownloadContext>,
    mc_version: &str,
    optifine_version: &str,
    has_forge: bool,
    ctx: &TaskContext,
) -> Result<OptifineFiles> {
    let listing = options
        .game
        .source
        .fetch_optifine_versions(mc_version)
        .await?;
    let entry = listing
        .into_iter()
        .find(|entry| entry.version() == optifine_version)
        .ok_or_else(|| {
            anyhow::anyhow!("Minecraft {mc_version} 没有 OptiFine 版本 {optifine_version}")
        })?;
    let library_version = entry.library_version(mc_version);
    let path = libraries_dir(&options.root_path).join(lib_name_to_path(&format!(
        "{OPTIFINE_LIBRARY}:{library_version}"
    )));
    ctx.log(format!(
        "OptiFine {} → {}",
        entry.version(),
        base_name(&path)
    ));

    let name = if entry.filename.is_empty() {
        base_name(&path)
    } else {
        entry.filename.clone()
    };
    download_planned(
        &options.game,
        downloads,
        "OptiFine",
        vec![PlannedFile {
            url: Some(entry.download_url()),
            save_to: path.clone(),
            checksum: None,
            size: None,
            name,
        }],
        ctx,
    )
    .await?;

    let launchwrapper = if has_forge {
        ctx.log("Forge 已提供 launchwrapper，跳过 OptiFine 自带的版本");
        None
    } else {
        let bytes = tokio::fs::read(&path)
            .await
            .with_context(|| format!("无法读取 {}", path.display()))?;
        let mut archive = open_archive(bytes)?;
        let version = launchwrapper_version_from_archive(&mut archive)?;
        validate_launchwrapper_version(&version)?;

        let inner = format!("launchwrapper-of-{version}.jar");
        let bytes = read_entry(&mut archive, &inner)?;
        let target = libraries_dir(&options.root_path).join(lib_name_to_path(&format!(
            "{LAUNCHWRAPPER_LIBRARY}:of-{version}"
        )));
        write_safe(&target, &bytes).await?;
        ctx.log(format!("解出 {}", base_name(&target)));
        Some(version)
    };

    Ok(OptifineFiles {
        library_version,
        launchwrapper,
    })
}

/// 生成 OptiFine 的 patch。
///
/// * 没有 Forge：加 OptiFine + launchwrapper，主类改成 launchwrapper 的 `Launch`；
/// * 同时有 Forge：Forge 已经提供 launchwrapper，只加 OptiFine 库，主类保持 Forge 的，
///   tweaker 用 `OptiFineForgeTweaker`。
///
/// 真正的合并与写盘由安装图最后的「完成」步骤统一处理。
pub fn install_optifine(files: &OptifineFiles, has_forge: bool) -> VersionJsonPatch {
    let mut libraries = vec![Library {
        name: format!("{OPTIFINE_LIBRARY}:{}", files.library_version),
        ..Default::default()
    }];
    let main_class = if has_forge {
        None
    } else {
        if let Some(version) = &files.launchwrapper {
            libraries.push(Library {
                name: format!("{LAUNCHWRAPPER_LIBRARY}:of-{version}"),
                ..Default::default()
            });
        }
        Some("net.minecraft.launchwrapper.Launch".to_string())
    };
    let tweaker = if has_forge {
        "optifine.OptiFineForgeTweaker"
    } else {
        "optifine.OptiFineTweaker"
    };

    VersionJsonPatch {
        name: "optifine".to_string(),
        version: files.library_version.clone(),
        libraries,
        tweakers: vec![tweaker.to_string()],
        main_class,
        kind: None,
        release_time: None,
        arguments: None,
        extra: JsonMap::new(),
    }
}

/// 从 OptiFine jar 里读 launchwrapper 版本。
///
/// 优先读 `launchwrapper-of.txt`（内容是 `2.2` 这种），没有时退回扫描
/// `launchwrapper-of-<version>.jar` 的条目名。
fn launchwrapper_version_from_archive(
    archive: &mut zip::ZipArchive<Cursor<Vec<u8>>>,
) -> Result<String> {
    if has_entry(archive, "launchwrapper-of.txt") {
        let text = read_entry(archive, "launchwrapper-of.txt")?;
        let version = String::from_utf8_lossy(&text).trim().to_string();
        if !version.is_empty() {
            return Ok(version);
        }
    }

    let names: Vec<String> = archive
        .file_names()
        .filter_map(|name| name.ok().map(|name| name.into_owned()))
        .collect();
    for name in names {
        if let Some(version) = name
            .strip_prefix("launchwrapper-of-")
            .and_then(|rest| rest.strip_suffix(".jar"))
        {
            return Ok(version.to_string());
        }
    }
    anyhow::bail!("OptiFine jar 里没有 launchwrapper")
}

/// launchwrapper 版本会拼进库路径，不能让文件名里的 `..` / 分隔符逃出目录。
fn validate_launchwrapper_version(version: &str) -> Result<()> {
    if version.is_empty()
        || !version
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_'))
    {
        anyhow::bail!("OptiFine jar 里的 launchwrapper 版本不安全：{version}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dmcl_mcmeta::vanilla::VersionJson;

    fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buffer = Vec::new();
        {
            let mut writer = zip::ZipWriter::new(Cursor::new(&mut buffer));
            for (name, data) in files {
                let options: zip::write::FileOptions<'_, '_, ()> =
                    zip::write::FileOptions::default();
                writer.start_file(*name, options).unwrap();
                std::io::Write::write_all(&mut writer, data).unwrap();
            }
            writer.finish().unwrap();
        }
        buffer
    }

    #[test]
    fn launchwrapper_version_prefers_the_text_file() {
        let inner = zip_of(&[("README", b"x")]);
        let bytes = zip_of(&[
            ("launchwrapper-of.txt", b"2.2\n"),
            ("launchwrapper-of-9.9.jar", &inner),
        ]);
        let mut archive = open_archive(bytes).unwrap();
        assert_eq!(
            launchwrapper_version_from_archive(&mut archive).unwrap(),
            "2.2"
        );
    }

    #[test]
    fn launchwrapper_version_falls_back_to_the_inner_jar_name() {
        let inner = zip_of(&[("README", b"x")]);
        let bytes = zip_of(&[("launchwrapper-of-2.3.jar", &inner)]);
        let mut archive = open_archive(bytes).unwrap();
        assert_eq!(
            launchwrapper_version_from_archive(&mut archive).unwrap(),
            "2.3"
        );
    }

    #[test]
    fn unsafe_launchwrapper_versions_are_rejected() {
        assert!(validate_launchwrapper_version("2.2").is_ok());
        assert!(validate_launchwrapper_version("../2.2").is_err());
        assert!(validate_launchwrapper_version("2/2").is_err());
    }

    #[test]
    fn vanilla_optifine_uses_launchwrapper_and_the_tweaker() {
        let mut json: VersionJson = serde_json::from_str(
            r#"{
                "id": "1.19",
                "mainClass": "net.minecraft.client.main.Main",
                "minecraftArguments": "--username ${auth_player_name}",
                "libraries": [{"name": "net.sf.jopt-simple:jopt-simple:5.0.4"}]
            }"#,
        )
        .unwrap();
        let files = OptifineFiles {
            library_version: "1.19_HD_U_H8".to_string(),
            launchwrapper: Some("2.2".to_string()),
        };

        let patch = install_optifine(&files, false);
        json.apply_patch(&patch).unwrap();

        assert_eq!(
            json.main_class.as_deref(),
            Some("net.minecraft.launchwrapper.Launch")
        );
        assert!(json
            .libraries
            .iter()
            .any(|library| library.name == "optifine:OptiFine:1.19_HD_U_H8"));
        assert!(json
            .libraries
            .iter()
            .any(|library| library.name == "net.minecraft:launchwrapper:of-2.2"));
        // 原有库原样保留，只在后面追加 OptiFine 的库。
        assert!(json
            .libraries
            .iter()
            .any(|library| library.name == "net.sf.jopt-simple:jopt-simple:5.0.4"));
        assert_eq!(
            json.minecraft_arguments.as_deref(),
            Some("--username ${auth_player_name} --tweakClass optifine.OptiFineTweaker")
        );
        assert!(json.arguments.is_none());
    }

    #[test]
    fn forge_optifine_keeps_the_forge_main_class_and_skips_launchwrapper() {
        let mut json: VersionJson = serde_json::from_str(
            r#"{
                "id": "1.19-forge",
                "mainClass": "cpw.mods.bootstraplauncher.BootstrapLauncher",
                "arguments": {"game": ["--username", "${auth_player_name}"]}
            }"#,
        )
        .unwrap();
        let files = OptifineFiles {
            library_version: "1.19_HD_U_H8".to_string(),
            launchwrapper: None,
        };

        let patch = install_optifine(&files, true);
        json.apply_patch(&patch).unwrap();

        assert_eq!(
            json.main_class.as_deref(),
            Some("cpw.mods.bootstraplauncher.BootstrapLauncher")
        );
        assert!(json
            .libraries
            .iter()
            .any(|library| library.name == "optifine:OptiFine:1.19_HD_U_H8"));
        assert!(!json
            .libraries
            .iter()
            .any(|library| library.name.starts_with("net.minecraft:launchwrapper")));

        let game = json.arguments.as_ref().unwrap().game.as_ref().unwrap();
        assert!(matches!(
            &game[game.len() - 2],
            dmcl_mcmeta::vanilla::Argument::Plain(value) if value == "--tweakClass"
        ));
        assert!(matches!(
            &game[game.len() - 1],
            dmcl_mcmeta::vanilla::Argument::Plain(value) if value == "optifine.OptiFineForgeTweaker"
        ));
    }
}
