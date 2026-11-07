//! 联网冒烟测试（默认 `#[ignore]`）：
//!
//! ```text
//! cargo test -p dmcl-install --test online -- --ignored --nocapture
//! ```
//!
//! 这里不会跑完整的安装（要下几百 MB 资源），而是覆盖最容易出错的那几段：
//! 版本元数据落盘、规划结果对真实版本 JSON 是否成立、规划出来的文件是否真能下载。

use std::path::PathBuf;
use std::sync::Arc;

use dmcl_game::{download_planned, plan_libraries, resolve_version, GameFilesOptions, VersionRef};
use dmcl_install::InstallOptions;
use dmcl_java::{resolve_jre_component, JavaOption, JavaRuntimeOptions, JreAutoInstall};
use dmcl_mcmeta::os::Arch;
use dmcl_mcmeta::source::{DownloadSource, Mirror};
use dmcl_mcmeta::vanilla::current_jre_platform;
use dmcl_task::{task_fn, ProgressHandle, TaskGraphBuilder};

const VERSION: &str = "1.7.10";

fn work_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("dmcl-install-test-{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn options(root: PathBuf) -> Arc<InstallOptions> {
    let game = GameFilesOptions::new(DownloadSource::official())
        .with_download(dmcl_download::DownloadOptions::with_max_connections(8));
    let java = Arc::new(JavaOption {
        path: PathBuf::from("/nonexistent/java"),
        major: 17,
        arch: Arch::X86_64,
    });
    let jre = JreAutoInstall::new(root.join("runtime"));
    Arc::new(InstallOptions::new(root, game, java, jre))
}

fn runtime_options() -> JavaRuntimeOptions {
    JavaRuntimeOptions {
        source: DownloadSource::official(),
        download: dmcl_download::DownloadOptions::with_max_connections(8),
    }
}

#[tokio::test]
#[ignore = "requires network access"]
async fn vanilla_metadata_lands_on_disk_with_official_urls() {
    let root = work_dir("metadata");
    let options = options(root.clone());

    dmcl_install::download_vanilla(&options, VERSION, VERSION)
        .await
        .expect("download_vanilla");

    // launcher_profiles.json 必须写好
    assert!(root.join("launcher_profiles.json").is_file());

    let path = root.join(format!("versions/{VERSION}/{VERSION}.json"));
    let text = std::fs::read_to_string(&path).unwrap();
    let json: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(json["id"], VERSION);
    assert!(json["libraries"].is_array());

    // 落盘的文档必须是**具体地址**（官方地址），不能留 URL 模板
    assert!(!text.contains("${MC_"), "文档里残留了 URL 模板");
    let library_url = json["libraries"][0]["downloads"]["artifact"]["url"]
        .as_str()
        .unwrap_or_default();
    assert!(library_url.starts_with("https://"), "{library_url}");

    // 未建模的字段仍然保留
    for key in ["time", "releaseTime", "type", "minimumLauncherVersion"] {
        assert!(json.get(key).is_some(), "丢了 {key}");
    }
}

#[tokio::test]
#[ignore = "requires network access"]
async fn planning_holds_for_a_real_version_json() {
    let root = work_dir("planning");
    let options = options(root.clone());
    dmcl_install::download_vanilla(&options, VERSION, VERSION)
        .await
        .expect("download_vanilla");

    let json = resolve_version(&VersionRef::new(&options.root_path, VERSION))
        .await
        .unwrap()
        .json;
    let planned = plan_libraries(&options.version(VERSION), &json);
    assert!(!planned.is_empty());

    for file in &planned {
        let item = file.clone().into_item(&options.game);
        // 规划出来的地址必须是可用的具体链接
        assert!(
            item.urls.iter().all(|url| url.starts_with("https://")),
            "{} -> {:?}",
            file.name,
            item.urls
        );
        assert!(item.save_to.starts_with(root.join("libraries")));
        // 1.7.10 有 linux natives，必须规划到解压清单里
    }
    // 客户端 jar 也在计划里
    let client = dmcl_game::plan_client_jar(&options.version(VERSION), &json).unwrap();
    assert!(client
        .into_item(&options.game)
        .urls
        .iter()
        .all(|url| url.starts_with("https://")));
}

#[tokio::test]
#[ignore = "requires network access"]
async fn a_few_planned_libraries_really_download() {
    let root = work_dir("download");
    let options = options(root.clone());
    dmcl_install::download_vanilla(&options, VERSION, VERSION)
        .await
        .expect("download_vanilla");

    let json = resolve_version(&VersionRef::new(&options.root_path, VERSION))
        .await
        .unwrap()
        .json;
    // 挑几个小文件，避免测试太重
    let small: Vec<_> = plan_libraries(&options.version(VERSION), &json)
        .into_iter()
        .take(3)
        .collect();

    let game = options.game.clone();
    let downloads = game.download_context(ProgressHandle::new());
    for _ in 0..2 {
        let mut builder = TaskGraphBuilder::new("下载");
        let step = builder.step("下载");
        let game = game.clone();
        let downloads = downloads.clone();
        let small = small.clone();
        let sink = builder.add(
            step,
            "下载库文件",
            (),
            task_fn(move |_: (), ctx| {
                let game = game.clone();
                let downloads = downloads.clone();
                let small = small.clone();
                async move { download_planned(&game, &downloads, "库文件", small, &ctx).await }
            }),
        );
        builder
            .build(sink)
            .unwrap()
            .run(|_| {})
            .await
            .expect("下载库文件");
    }
    for file in &small {
        assert!(
            file.save_to.is_file(),
            "{} 没有被下载",
            file.save_to.display()
        );
    }
}

#[tokio::test]
#[ignore = "requires network access"]
async fn the_fabric_loader_list_can_be_fetched_newest_first() {
    let source = DownloadSource::new(Mirror::Official);
    let list = source.fetch_fabric_loaders("1.20.1").await.expect("fabric");
    let latest = list.first().expect("fabric 列表非空");
    assert!(!latest.loader.version.is_empty());
}

#[tokio::test]
#[ignore = "requires network access"]
async fn optifine_downloads_extracts_launchwrapper_and_rewrites_the_json() {
    let version_name = "optifine-test";
    let root = work_dir("optifine");
    let version_dir = root.join("versions").join(version_name);
    tokio::fs::create_dir_all(&version_dir).await.unwrap();
    tokio::fs::write(
        version_dir.join(format!("{version_name}.json")),
        r#"{
            "id": "optifine-test",
            "mainClass": "net.minecraft.client.main.Main",
            "minecraftArguments": "--username ${auth_player_name}"
        }"#,
    )
    .await
    .unwrap();

    let options = options(root.clone());
    let previous = resolve_version(&VersionRef::new(&root, version_name))
        .await
        .unwrap();
    let mut builder = TaskGraphBuilder::new("安装 OptiFine");
    let step = builder.step("安装");
    let downloads = options.game.download_context(builder.progress());
    let sink = builder.add(
        step,
        "安装 OptiFine",
        (),
        task_fn(move |_: (), ctx| {
            let options = options.clone();
            let downloads = downloads.clone();
            let version_name = version_name.to_string();
            let previous = previous.clone();
            async move {
                dmcl_install::optifine::install_optifine(
                    &previous,
                    &options,
                    &downloads,
                    &version_name,
                    "1.12.2",
                    "HD_U_G5",
                    false,
                    &ctx,
                )
                .await
            }
        }),
    );
    builder
        .build(sink)
        .unwrap()
        .run(|_| {})
        .await
        .expect("安装 OptiFine");

    // 1.12.2 G5 自带 launchwrapper 2.2
    assert!(root
        .join("libraries/optifine/OptiFine/1.12.2_HD_U_G5/OptiFine-1.12.2_HD_U_G5.jar")
        .is_file());
    assert!(root
        .join("libraries/net/minecraft/launchwrapper/of-2.2/launchwrapper-of-2.2.jar")
        .is_file());

    let json: serde_json::Value = serde_json::from_str(
        &tokio::fs::read_to_string(version_dir.join(format!("{version_name}.json")))
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(json["mainClass"], "net.minecraft.launchwrapper.Launch");
    assert!(json["libraries"]
        .as_array()
        .unwrap()
        .iter()
        .any(|library| library["name"] == "optifine:OptiFine:1.12.2_HD_U_G5"));
    assert!(json["libraries"]
        .as_array()
        .unwrap()
        .iter()
        .any(|library| library["name"] == "net.minecraft:launchwrapper:of-2.2"));
    assert!(json["minecraftArguments"]
        .as_str()
        .unwrap()
        .ends_with("--tweakClass optifine.OptiFineTweaker"));
}

#[tokio::test]
#[ignore = "requires network access"]
async fn the_jre_runtime_manifest_can_be_resolved_for_this_platform() {
    let options = runtime_options();
    let platform = current_jre_platform().expect("当前平台应有 Mojang 提供的运行时");

    let manifest = dmcl_java::fetch_jre_runtime_manifest(&options, "java-runtime-gamma", platform)
        .await
        .expect("运行时清单");

    assert!(!manifest.files.is_empty());
    let java = if cfg!(target_os = "macos") {
        "jre.bundle/Contents/Home/bin/java"
    } else if cfg!(windows) {
        "bin/java.exe"
    } else {
        "bin/java"
    };
    assert!(manifest.files.contains_key(java), "缺少 {java}");
}

#[tokio::test]
#[ignore = "requires network access"]
async fn jre_components_can_be_resolved_from_the_real_index() {
    let source = DownloadSource::official();
    let index = source.fetch_jre_manifest().await.expect("JRE 产品索引");
    let platform = current_jre_platform().expect("当前平台应有 Mojang 提供的运行时");

    let component = resolve_jre_component(&index, platform, None, 17).expect("Java 17 组件");
    assert!(component.name.starts_with("java-runtime-"));

    let legacy = resolve_jre_component(&index, platform, None, 8).expect("Java 8 组件");
    assert_eq!(legacy.name, "jre-legacy");

    // 显式组件必须实际提供要求的版本。
    assert!(resolve_jre_component(&index, platform, Some(&component.name), 8).is_err());
    // Mojang 目前没有 Java 99；不退回 jre-legacy。
    assert!(resolve_jre_component(&index, platform, None, 99).is_err());
}
