//! OptiFine 的安装（查询见 [`dmcl_mcmeta::optifine`]）。

use std::io::Cursor;
use std::sync::Arc;

use anyhow::{Context, Result};
use dmcl_download::DownloadContext;
use dmcl_game::{base_name, download_planned, PlannedFile, ResolvedVersion};
use dmcl_mcmeta::layout::{libraries_dir, version_json_path};
use dmcl_mcmeta::maven::lib_name_to_path;
use dmcl_mcmeta::vanilla::{Argument, Library, VersionJson};
use dmcl_task::{task_fn, Handle, StepId, TaskContext, TaskGraphBuilder};

use crate::archive::{has_entry, open_archive, read_entry};
use crate::io::{write_json_safe, write_safe};
use crate::loader::common::finish_version_json;
use crate::options::InstallOptions;

/// OptiFine 的库坐标前缀。
const OPTIFINE_LIBRARY: &str = "optifine:OptiFine";
/// launchwrapper 的库坐标前缀。
const LAUNCHWRAPPER_LIBRARY: &str = "net.minecraft:launchwrapper";
/// 游戏参数里的 tweakClass 开关。
const TWEAK_CLASS: &str = "--tweakClass";

/// 下载 OptiFine 得到的产物：写版本 JSON 只需要这里的版本号。
#[derive(Debug, Clone)]
pub struct OptifineFiles {
    /// `optifine:OptiFine:<version>` 里的版本部分。
    pub library_version: String,
    /// 从 jar 里解出的 launchwrapper 版本；有 Forge 时为 `None`。
    pub launchwrapper: Option<String>,
}

/// 往安装图里追加 OptiFine：先并行下载 jar，再等前面的加载器合并完版本 JSON。
///
/// 「下载 OptiFine」只依赖版本列表与共享的 `libraries/`，因此和原版元数据、版本 jar、
/// 资源、Forge 安装同时起跑；改版本 JSON 的节点依赖 `previous`，确保 Forge 的
/// `mainClass` / launchwrapper 已经就位。
#[expect(clippy::too_many_arguments)]
pub(crate) fn add_optifine_install(
    builder: &mut TaskGraphBuilder,
    step: StepId,
    options: Arc<InstallOptions>,
    downloads: Arc<DownloadContext>,
    previous: Handle<ResolvedVersion>,
    version_name: String,
    mc_version: String,
    optifine_version: String,
    has_forge: bool,
) -> Handle<ResolvedVersion> {
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
        "安装 OptiFine",
        (previous, files),
        task_fn({
            let version_name = version_name.clone();
            move |(previous, files): (Arc<ResolvedVersion>, Arc<OptifineFiles>),
                  ctx: TaskContext| {
                let version_name = version_name.clone();
                async move {
                    install_optifine(&previous, &files, &version_name, has_forge, &ctx).await
                }
            }
        }),
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
            url: entry.download_url(),
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

/// 把 OptiFine 写进版本 JSON。
///
/// 这一步要等前面的加载器（尤其是 Forge）合并完版本 JSON，才能在它们的 `mainClass` /
/// launchwrapper 之上追加 OptiFine 的库与 tweaker。
pub async fn install_optifine(
    previous: &ResolvedVersion,
    files: &OptifineFiles,
    version_name: &str,
    has_forge: bool,
    ctx: &TaskContext,
) -> Result<ResolvedVersion> {
    let mut json = previous.json.clone();
    apply_optifine_version_json(
        &mut json,
        &files.library_version,
        files.launchwrapper.as_deref(),
        has_forge,
    );
    finish_version_json(&mut json, version_name);
    write_json_safe(
        &version_json_path(&previous.reference.root, version_name),
        &json,
    )
    .await?;
    ctx.log("OptiFine 已写入版本元数据");
    Ok(ResolvedVersion {
        reference: previous.reference.clone(),
        json,
    })
}

/// 把 OptiFine 的库与 tweakClass 写进版本 JSON。
///
/// * 没有 Forge：加 OptiFine + launchwrapper，主类改成 launchwrapper 的 `Launch`；
/// * 同时有 Forge：Forge 已经提供 launchwrapper，只加 OptiFine 库，主类保持 Forge 的，
///   tweakClass 用 `OptiFineForgeTweaker`。
fn apply_optifine_version_json(
    json: &mut VersionJson,
    optifine_library_version: &str,
    launchwrapper: Option<&str>,
    has_forge: bool,
) {
    json.libraries.push(Library {
        name: format!("{OPTIFINE_LIBRARY}:{optifine_library_version}"),
        ..Default::default()
    });

    if !has_forge {
        if let Some(version) = launchwrapper {
            json.libraries.push(Library {
                name: format!("{LAUNCHWRAPPER_LIBRARY}:of-{version}"),
                ..Default::default()
            });
        }
        json.main_class = Some("net.minecraft.launchwrapper.Launch".to_string());
    }

    let tweaker = if has_forge {
        "optifine.OptiFineForgeTweaker"
    } else {
        "optifine.OptiFineTweaker"
    };
    append_game_tweak(json, tweaker);
}

/// 把 `--tweakClass <tweaker>` 追加到游戏参数里。
///
/// 现代版本写在 `arguments.game`；1.12.2 及更早没有 `arguments`，写在
/// `minecraftArguments` 字符串末尾。
fn append_game_tweak(json: &mut VersionJson, tweaker: &str) {
    if let Some(arguments) = json.arguments.as_mut() {
        let game = arguments.game.get_or_insert_with(Vec::new);
        game.push(Argument::Plain(TWEAK_CLASS.to_string()));
        game.push(Argument::Plain(tweaker.to_string()));
        return;
    }

    let arguments = json.minecraft_arguments.get_or_insert_with(String::new);
    if !arguments.is_empty() && !arguments.chars().last().is_some_and(char::is_whitespace) {
        arguments.push(' ');
    }
    arguments.push_str(TWEAK_CLASS);
    arguments.push(' ');
    arguments.push_str(tweaker);
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
                "libraries": [
                    {"name": "net.minecraft:launchwrapper:1.5"},
                    {"name": "net.sf.jopt-simple:jopt-simple:5.0.4"}
                ]
            }"#,
        )
        .unwrap();
        apply_optifine_version_json(&mut json, "1.19_HD_U_H8", Some("2.2"), false);

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
            .any(|library| library.name == "net.minecraft:launchwrapper:1.5"));
        assert!(json
            .libraries
            .iter()
            .any(|library| library.name == "net.sf.jopt-simple:jopt-simple:5.0.4"));
        assert_eq!(
            json.minecraft_arguments.as_deref(),
            Some("--username ${auth_player_name} --tweakClass optifine.OptiFineTweaker")
        );
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
        apply_optifine_version_json(&mut json, "1.19_HD_U_H8", None, true);

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
            Argument::Plain(value) if value == TWEAK_CLASS
        ));
        assert!(matches!(
            &game[game.len() - 1],
            Argument::Plain(value) if value == "optifine.OptiFineForgeTweaker"
        ));
    }
}
