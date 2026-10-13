//! 联网冒烟测试（默认 `#[ignore]`）：
//!
//! ```text
//! cargo test -p dmcl-download --test online -- --ignored --nocapture
//! ```

use std::path::PathBuf;

use dmcl_download::{
    hash_file, DownloadItem, DownloadOptions, DownloadReport, DownloadTask, HashAlgorithm,
};
use dmcl_task::{TaskChain, TaskContext, TaskEvent};

/// 一个小而稳定的官方文件（约 200 KiB 的版本清单）。
const URL: &str = "https://piston-meta.mojang.com/mc/game/version_manifest.json";

fn work_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("dmcl-download-test-{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// 直接跑一次并拿到报告（不要进度界面）。
async fn execute(task: &DownloadTask) -> DownloadReport {
    task.execute(&TaskContext::detached("下载"))
        .await
        .expect("下载执行失败")
}

#[tokio::test]
#[ignore = "requires network access"]
async fn single_stream_download_writes_the_file() {
    let dir = work_dir("single");
    let target = dir.join("manifest.json");

    let task = DownloadTask::new(DownloadOptions::default().without_chunking())
        .with_item(DownloadItem::new([URL.to_string()], &target));

    let report = execute(&task).await;
    assert!(report.is_success(), "{:?}", report.failed);
    assert_eq!(report.downloaded, 1);
    assert_eq!(report.skipped, 0);

    let text = std::fs::read_to_string(&target).unwrap();
    assert!(text.contains("latest"));
    // 临时文件必须已经被改名掉
    assert!(!target.with_file_name("manifest.json.part").exists());

    // 再跑一遍：文件已是最新，应当被跳过
    let again = execute(&task).await;
    assert_eq!(again.skipped, 1);
    assert_eq!(again.downloaded, 0);
}

#[tokio::test]
#[ignore = "requires network access"]
async fn chunked_download_matches_single_stream_byte_for_byte() {
    let dir = work_dir("chunked");
    let single = dir.join("single.json");
    let chunked = dir.join("chunked.json");

    let first = DownloadTask::new(DownloadOptions::default().without_chunking())
        .with_item(DownloadItem::new([URL.to_string()], &single));
    assert!(execute(&first).await.is_success());

    let size = std::fs::metadata(&single).unwrap().len();
    // 这里特意用 sha256：顺带验证非 sha1 算法在真实下载里也能走通
    let sha256 = hash_file(&single, HashAlgorithm::Sha256).await.unwrap();

    // 强制分块：把阈值和块大小都调小，让一个小文件也真的被切成好几段
    let options = DownloadOptions::default().with_chunking(1024, 4096);
    let second = DownloadTask::new(options).with_item(
        DownloadItem::new([URL.to_string()], &chunked)
            .with_size(Some(size))
            .with_sha256(Some(sha256)),
    );
    let report = execute(&second).await;
    assert!(report.is_success(), "{:?}", report.failed);

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

    let task = DownloadTask::new(DownloadOptions::default().without_chunking())
        .with_item(DownloadItem::new([URL.to_string()], &target));
    let chain = TaskChain::new("下载").then(task, "下载文件");

    let mut progress_events = 0usize;
    chain
        .run_with((), |event| {
            if let TaskEvent::Progress { .. } = event {
                progress_events += 1;
            }
        })
        .await
        .unwrap();

    assert!(progress_events >= 2, "应当至少上报开始与结束两次进度");
}

#[tokio::test]
#[ignore = "requires network access"]
async fn a_bad_url_is_reported_but_other_files_still_finish() {
    let dir = work_dir("failure");
    let good = dir.join("manifest.json");
    let bad = dir.join("missing.json");

    let mut task = DownloadTask::new(
        DownloadOptions::default()
            .without_chunking()
            .with_max_attempts(1),
    )
    .with_item(DownloadItem::new([URL.to_string()], &good));
    task.push(DownloadItem::new(
        ["https://piston-meta.mojang.com/mc/game/definitely-not-here.json".to_string()],
        &bad,
    ));

    let report = execute(&task).await;
    assert_eq!(report.downloaded, 1);
    assert_eq!(report.failed.len(), 1);
    assert!(report.failure_summary(5).contains("missing.json"));
    // 失败的文件不能留下半成品
    assert!(!bad.exists());
    assert!(!bad.with_file_name("missing.json.part").exists());
}
