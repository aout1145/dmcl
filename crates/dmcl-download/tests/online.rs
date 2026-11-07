//! 联网冒烟测试（默认 `#[ignore]`）：
//!
//! ```text
//! cargo test -p dmcl-download --test online -- --ignored --nocapture
//! ```

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use dmcl_download::{hash_file, DownloadContext, DownloadItem, DownloadOptions, HashAlgorithm};
use dmcl_task::{task_fn, ProgressHandle, TaskEvent, TaskGraphBuilder};

/// 一个小而稳定的官方文件（约 200 KiB 的版本清单）。
const URL: &str = "https://piston-meta.mojang.com/mc/game/version_manifest.json";

fn work_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("dmcl-download-test-{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn downloads(options: DownloadOptions) -> Arc<DownloadContext> {
    DownloadContext::new(reqwest::Client::new(), options, ProgressHandle::new())
}

async fn run_items(
    downloads: Arc<DownloadContext>,
    items: Vec<DownloadItem>,
) -> (anyhow::Result<Arc<()>>, Vec<TaskEvent>) {
    let mut builder = TaskGraphBuilder::new("下载");
    let step = builder.step("下载");
    let sink = builder.add(
        step,
        "准备下载",
        (),
        task_fn(move |_: (), ctx| {
            let downloads = downloads.clone();
            let items = items.clone();
            async move { downloads.prepare(&ctx, step, items).await.map(|_| ()) }
        }),
    );
    let graph = builder.build(sink).unwrap();
    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let collected = events.clone();
    let result = graph
        .run(move |event| collected.lock().unwrap().push(event))
        .await;
    let events = events.lock().unwrap().clone();
    (result, events)
}

#[tokio::test]
#[ignore = "requires network access"]
async fn single_stream_download_writes_the_file() {
    let dir = work_dir("single");
    let target = dir.join("manifest.json");

    let items = vec![DownloadItem::new([URL.to_string()], &target)];
    let (result, _) = run_items(
        downloads(DownloadOptions::default().without_chunking()),
        items.clone(),
    )
    .await;
    result.unwrap();

    let text = std::fs::read_to_string(&target).unwrap();
    assert!(text.contains("latest"));
    assert!(!target.with_file_name("manifest.json.part").exists());

    // 再跑一遍：文件已是最新，预检直接跳过，不会再发请求。
    let (result, events) = run_items(
        downloads(DownloadOptions::default().without_chunking()),
        items,
    )
    .await;
    result.unwrap();
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, TaskEvent::TaskAdded { .. })),
        "{events:?}"
    );
}

#[tokio::test]
#[ignore = "requires network access"]
async fn chunked_download_matches_single_stream_byte_for_byte() {
    let dir = work_dir("chunked");
    let single = dir.join("single.json");
    let chunked = dir.join("chunked.json");

    let (result, _) = run_items(
        downloads(DownloadOptions::default().without_chunking()),
        vec![DownloadItem::new([URL.to_string()], &single)],
    )
    .await;
    result.unwrap();

    let size = std::fs::metadata(&single).unwrap().len();
    // 这里特意用 sha256：顺带验证非 sha1 算法在真实下载里也能走通
    let sha256 = hash_file(&single, HashAlgorithm::Sha256).await.unwrap();

    let options = DownloadOptions::default().with_chunking(1024, 4096);
    let (result, _) = run_items(
        downloads(options),
        vec![DownloadItem::new([URL.to_string()], &chunked)
            .with_size(Some(size))
            .with_sha256(Some(sha256))],
    )
    .await;
    result.unwrap();

    assert_eq!(
        std::fs::read(&single).unwrap(),
        std::fs::read(&chunked).unwrap()
    );
}

#[tokio::test]
#[ignore = "requires network access"]
async fn progress_events_are_emitted_while_downloading() {
    let dir = work_dir("progress");
    let target = dir.join("manifest.json");

    let items = vec![DownloadItem::new([URL.to_string()], &target)];
    let (result, events) = run_items(
        downloads(DownloadOptions::default().without_chunking()),
        items,
    )
    .await;
    result.unwrap();

    assert!(
        events
            .iter()
            .any(|event| matches!(event, TaskEvent::Progress { .. })),
        "至少要有一条最终进度快照"
    );
}

#[tokio::test]
#[ignore = "requires network access"]
async fn a_bad_url_fails_the_graph() {
    let dir = work_dir("failure");
    let server = DownloadOptions::default()
        .without_chunking()
        .with_max_retries(0);

    let items = vec![DownloadItem::new(
        ["https://piston-meta.mojang.com/mc/game/definitely-not-here.json".to_string()],
        dir.join("missing.json"),
    )];
    let (result, _) = run_items(downloads(server), items).await;
    result.unwrap_err();
    assert!(!dir.join("missing.json").exists());
    assert!(!dir.join("missing.json.part").exists());
}

#[tokio::test]
#[ignore = "requires network access"]
async fn a_slow_download_can_be_cancelled() {
    let dir = work_dir("cancel");
    let mut builder = TaskGraphBuilder::new("下载");
    let step = builder.step("下载");
    let downloads = downloads(DownloadOptions::default().without_chunking());
    let sink = builder.add(
        step,
        "下载",
        (),
        downloads.task(DownloadItem::new(
            [URL.to_string()],
            dir.join("manifest.json"),
        )),
    );
    let graph = builder.build(sink).unwrap();
    let cancel = graph.cancel_handle();
    let handle = tokio::spawn(async move { graph.run(|_| {}).await });
    tokio::time::sleep(Duration::from_millis(1)).await;
    cancel.cancel();
    // 取消可能发生在下载完成之后；这里只要求一定在时限内结束。
    let _ = tokio::time::timeout(Duration::from_secs(5), handle)
        .await
        .expect("取消应当及时结束");
}
