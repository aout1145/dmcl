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

use dmcl_install::{download_planned, plan_libraries, InstallOptions};
use dmcl_mcmeta::{DownloadSource, Mirror};
use dmcl_task::TaskContext;

const VERSION: &str = "1.7.10";

fn work_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("dmcl-install-test-{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn options(root: PathBuf) -> Arc<InstallOptions> {
    Arc::new(
        InstallOptions::new(root, DownloadSource::official())
            .with_download(dmcl_download::DownloadOptions::with_max_connections(8)),
    )
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

    let json = options.version_json(VERSION).await.unwrap();
    let planned = plan_libraries(&options.plan(), &json);
    assert!(!planned.is_empty());

    for file in &planned {
        let item = file.clone().into_item(&options);
        // 规划出来的地址必须是可用的具体链接
        assert!(
            item.url.starts_with("https://"),
            "{} -> {}",
            file.name,
            item.url
        );
        assert!(item.save_to.starts_with(root.join("libraries")));
        // 1.7.10 有 linux natives，必须规划到解压清单里
    }
    // 客户端 jar 也在计划里
    let client = dmcl_install::plan_client_jar(&options.plan(), &json, VERSION).unwrap();
    assert!(client.into_item(&options).url.starts_with("https://"));
}

#[tokio::test]
#[ignore = "requires network access"]
async fn a_few_planned_libraries_really_download() {
    let root = work_dir("download");
    let options = options(root.clone());
    dmcl_install::download_vanilla(&options, VERSION, VERSION)
        .await
        .expect("download_vanilla");

    let json = options.version_json(VERSION).await.unwrap();
    // 挑几个小文件，避免测试太重
    let small: Vec<_> = plan_libraries(&options.plan(), &json)
        .into_iter()
        .take(3)
        .collect();

    let report = download_planned(
        &options,
        "库文件",
        small.clone(),
        &TaskContext::detached("测试"),
    )
    .await
    .expect("下载库文件");
    assert_eq!(report.failed.len(), 0);
    for file in &small {
        assert!(
            file.save_to.is_file(),
            "{} 没有被下载",
            file.save_to.display()
        );
    }

    // 再跑一次：应当全部被跳过（sha1 一致）
    let again = download_planned(&options, "库文件", small, &TaskContext::detached("测试"))
        .await
        .unwrap();
    assert_eq!(again.skipped, 3);
    assert_eq!(again.downloaded, 0);
}

#[tokio::test]
#[ignore = "requires network access"]
async fn the_fabric_loader_list_reports_a_latest_version() {
    let source = DownloadSource::new(Mirror::Official);
    let latest = dmcl_install::latest_fabric_loader(&source, "1.20.1")
        .await
        .expect("fabric");
    assert!(!latest.is_empty());
}
