//! 本地 HTTP 测试服务器与下载并发 / 重试 / 取消测试。

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dmcl_download::{hash_bytes, DownloadContext, DownloadItem, DownloadOptions, HashAlgorithm};
use dmcl_task::{task_fn, ProgressHandle, TaskEvent, TaskGraph, TaskGraphBuilder};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

// ---------------------------------------------------------------------------
// 本地 HTTP 服务器
// ---------------------------------------------------------------------------

struct TestServer {
    addr: SocketAddr,
    active: Arc<AtomicUsize>,
    max_active: Arc<AtomicUsize>,
    requests: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl TestServer {
    async fn start(files: HashMap<String, Vec<u8>>, delay: Duration) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let files = Arc::new(files);
        let active = Arc::new(AtomicUsize::new(0));
        let max_active = Arc::new(AtomicUsize::new(0));
        let requests = Arc::new(AtomicUsize::new(0));

        let task = {
            let active = active.clone();
            let max_active = max_active.clone();
            let requests = requests.clone();
            tokio::spawn(async move {
                loop {
                    let Ok((stream, _)) = listener.accept().await else {
                        break;
                    };
                    let files = files.clone();
                    let active = active.clone();
                    let max_active = max_active.clone();
                    let requests = requests.clone();
                    tokio::spawn(async move {
                        serve(stream, files, active, max_active, requests, delay).await;
                    });
                }
            })
        };

        Self {
            addr,
            active,
            max_active,
            requests,
            task,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }

    fn max_active(&self) -> usize {
        self.max_active.load(Ordering::SeqCst)
    }

    fn requests(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }

    #[allow(dead_code)]
    fn active(&self) -> usize {
        self.active.load(Ordering::SeqCst)
    }
}

/// 按脚本依次返回原始 HTTP 响应；脚本用完后重复最后一条。
struct ScriptedServer {
    addr: SocketAddr,
    requests: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for ScriptedServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl ScriptedServer {
    async fn start(responses: Vec<Vec<u8>>) -> Self {
        assert!(!responses.is_empty());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let responses = Arc::new(responses);
        let requests = Arc::new(AtomicUsize::new(0));

        let task = {
            let requests = requests.clone();
            tokio::spawn(async move {
                loop {
                    let Ok((mut stream, _)) = listener.accept().await else {
                        break;
                    };
                    let responses = responses.clone();
                    let requests = requests.clone();
                    tokio::spawn(async move {
                        if read_request(&mut stream).await.is_none() {
                            return;
                        }
                        let index = requests.fetch_add(1, Ordering::SeqCst);
                        let response = &responses[index.min(responses.len() - 1)];
                        let _ = stream.write_all(response).await;
                        let _ = stream.flush().await;
                    });
                }
            })
        };

        Self {
            addr,
            requests,
            task,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }

    fn requests(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }
}

/// 读一个请求，返回（含请求头的缓冲区，请求头结束位置）。
async fn read_request(stream: &mut TcpStream) -> Option<(Vec<u8>, usize)> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 1024];
    let header_end = loop {
        let read = match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => return None,
            Ok(read) => read,
        };
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(position) = find_subsequence(&buffer, b"\r\n\r\n") {
            break position + 4;
        }
        if buffer.len() > 64 * 1024 {
            return None;
        }
    };
    Some((buffer, header_end))
}

/// 拼一个带响应体的 HTTP 响应。
fn http_response(status: &str, headers: &[(&str, &str)], body: &[u8]) -> Vec<u8> {
    let mut response = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for (name, value) in headers {
        response.push_str(&format!("{name}: {value}\r\n"));
    }
    response.push_str("\r\n");
    let mut response = response.into_bytes();
    response.extend_from_slice(body);
    response
}

async fn serve(
    mut stream: TcpStream,
    files: Arc<HashMap<String, Vec<u8>>>,
    active: Arc<AtomicUsize>,
    max_active: Arc<AtomicUsize>,
    requests: Arc<AtomicUsize>,
    delay: Duration,
) {
    let Some((buffer, header_end)) = read_request(&mut stream).await else {
        return;
    };

    requests.fetch_add(1, Ordering::SeqCst);
    let current = active.fetch_add(1, Ordering::SeqCst) + 1;
    max_active.fetch_max(current, Ordering::SeqCst);
    if !delay.is_zero() {
        tokio::time::sleep(delay).await;
    }

    let request = String::from_utf8_lossy(&buffer[..header_end]);
    let path = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/");
    let range = request.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        if name.eq_ignore_ascii_case("range") {
            Some(value.trim().to_string())
        } else {
            None
        }
    });

    let response = match files.get(path) {
        None => {
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
        }
        Some(body) => match range
            .as_deref()
            .and_then(|range| parse_range(range, body.len()))
        {
            Some((start, end)) => {
                let slice = &body[start..=end];
                let mut response = format!(
                    "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {start}-{end}/{}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n",
                    slice.len(),
                    body.len()
                )
                .into_bytes();
                response.extend_from_slice(slice);
                response
            }
            None => {
                let mut response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .into_bytes();
                response.extend_from_slice(body);
                response
            }
        },
    };

    let _ = stream.write_all(&response).await;
    let _ = stream.flush().await;
    active.fetch_sub(1, Ordering::SeqCst);
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn parse_range(value: &str, length: usize) -> Option<(usize, usize)> {
    let value = value.trim().strip_prefix("bytes=")?;
    let (start, end) = value.split_once('-')?;
    let start: usize = start.parse().ok()?;
    let end = if end.is_empty() {
        length.checked_sub(1)?
    } else {
        end.parse().ok()?
    };
    if start >= length || end >= length || start > end {
        return None;
    }
    Some((start, end))
}

// ---------------------------------------------------------------------------
// 测试辅助
// ---------------------------------------------------------------------------

fn work_dir(tag: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("dmcl-download-local-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn downloads(options: DownloadOptions, progress: ProgressHandle) -> Arc<DownloadContext> {
    DownloadContext::new(reqwest::Client::new(), options, progress)
}

async fn run_items(
    options: DownloadOptions,
    items: Vec<DownloadItem>,
) -> (ProgressHandle, anyhow::Result<Arc<()>>, Vec<TaskEvent>) {
    let mut builder = TaskGraphBuilder::new("下载");
    let step = builder.step("下载");
    let sink = builder.add(
        step,
        "准备下载",
        (),
        task_fn(move |_: (), ctx| {
            let options = options.clone();
            let items = items.clone();
            async move {
                let downloads =
                    DownloadContext::new(reqwest::Client::new(), options, ctx.progress_handle());
                downloads.prepare(&ctx, step, items).await.map(|_| ())
            }
        }),
    );
    let graph: TaskGraph<()> = builder.build(sink).unwrap();
    let progress = graph.progress();
    let events = Arc::new(Mutex::new(Vec::new()));
    let collected = events.clone();
    let result = graph
        .run(move |event| collected.lock().unwrap().push(event))
        .await;
    let events = events.lock().unwrap().clone();
    (progress, result, events)
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_file_downloads_and_retries_with_the_next_candidate() {
    let dir = work_dir("retry");
    let body = b"hello from the second candidate".to_vec();
    let server = TestServer::start(
        HashMap::from([("/good".to_string(), body.clone())]),
        Duration::ZERO,
    )
    .await;

    let item = DownloadItem::new(
        [server.url("/missing"), server.url("/good")],
        dir.join("a.txt"),
    )
    .with_size(Some(body.len() as u64))
    .with_sha1(Some(hash_bytes(HashAlgorithm::Sha1, &body)));
    let options = DownloadOptions::default()
        .without_chunking()
        .with_max_retries(1);

    let (progress, result, _) = run_items(options, vec![item.clone()]).await;
    result.unwrap();

    assert_eq!(std::fs::read(dir.join("a.txt")).unwrap(), body);
    assert_eq!(progress.snapshot().downloaded, body.len() as u64);
    assert_eq!(server.requests(), 2, "第一次 404，第二次换源成功");
}

#[tokio::test]
async fn duplicate_targets_share_one_download() {
    let dir = work_dir("duplicate-target");
    let body = b"only downloaded once".to_vec();
    let server = TestServer::start(
        HashMap::from([("/same".to_string(), body.clone())]),
        Duration::ZERO,
    )
    .await;

    let item = DownloadItem::new([server.url("/same")], dir.join("same.bin"))
        .with_size(Some(body.len() as u64))
        .with_sha1(Some(hash_bytes(HashAlgorithm::Sha1, &body)));
    let options = DownloadOptions::default().without_chunking();

    let (progress, result, _) = run_items(options, vec![item.clone(), item]).await;
    result.unwrap();

    assert_eq!(std::fs::read(dir.join("same.bin")).unwrap(), body);
    assert_eq!(server.requests(), 1, "同一个目标路径只应请求一次");
    assert_eq!(progress.snapshot().total, body.len() as u64);
    assert_eq!(progress.snapshot().downloaded, body.len() as u64);
}

#[tokio::test]
async fn a_429_response_honours_retry_after() {
    let dir = work_dir("retry-after");
    let body = b"after the limit".to_vec();
    let server = ScriptedServer::start(vec![
        http_response("429 Too Many Requests", &[("Retry-After", "1")], b""),
        http_response("200 OK", &[], &body),
    ])
    .await;

    let item = DownloadItem::new([server.url("/limited")], dir.join("a.txt"))
        .with_size(Some(body.len() as u64))
        .with_sha1(Some(hash_bytes(HashAlgorithm::Sha1, &body)));
    // 指数退避故意设得比 Retry-After 短：确认等的是响应头。
    let options = DownloadOptions::default()
        .without_chunking()
        .with_max_retries(1)
        .with_retry_delay(Duration::from_millis(10), Duration::from_millis(10));

    let started = Instant::now();
    let (_, result, _) = run_items(options, vec![item]).await;
    result.unwrap();

    assert!(
        started.elapsed() >= Duration::from_millis(900),
        "应当等待 Retry-After 给出的 1 秒"
    );
    assert_eq!(server.requests(), 2);
    assert_eq!(std::fs::read(dir.join("a.txt")).unwrap(), body);
}

#[tokio::test]
async fn bmclapi_rate_limit_header_is_used_when_retry_after_is_missing() {
    let dir = work_dir("rate-limit-reset");
    let body = b"ok".to_vec();
    let server = ScriptedServer::start(vec![
        http_response(
            "429 Too Many Requests",
            &[("x-ratelimit-reset-short", "0")],
            b"",
        ),
        http_response("200 OK", &[], &body),
    ])
    .await;

    let item = DownloadItem::new([server.url("/limited")], dir.join("b.txt"))
        .with_size(Some(body.len() as u64))
        .with_sha1(Some(hash_bytes(HashAlgorithm::Sha1, &body)));
    // 如果没读到 bmclapi 的限流头，就会退避 5 秒。
    let options = DownloadOptions::default()
        .without_chunking()
        .with_max_retries(1)
        .with_retry_delay(Duration::from_secs(5), Duration::from_secs(5));

    let started = Instant::now();
    let (_, result, _) = run_items(options, vec![item]).await;
    result.unwrap();

    assert!(
        started.elapsed() < Duration::from_secs(2),
        "x-ratelimit-reset-short 应当覆盖指数退避"
    );
    assert_eq!(server.requests(), 2);
    assert_eq!(std::fs::read(dir.join("b.txt")).unwrap(), body);
}

#[tokio::test]
async fn transient_errors_back_off_exponentially() {
    let dir = work_dir("backoff");
    let server =
        ScriptedServer::start(vec![http_response("500 Internal Server Error", &[], b"")]).await;

    let item = DownloadItem::new([server.url("/broken")], dir.join("c.bin"));
    let options = DownloadOptions::default()
        .without_chunking()
        .with_max_retries(2)
        .with_retry_delay(Duration::from_millis(100), Duration::from_secs(10));

    let started = Instant::now();
    let (_, result, _) = run_items(options, vec![item]).await;
    result.unwrap_err();

    // 第 1 次重试等 100ms，第 2 次等 200ms。
    assert!(
        started.elapsed() >= Duration::from_millis(280),
        "两次重试必须按指数退避等待"
    );
    assert_eq!(server.requests(), 3);
}

#[tokio::test]
async fn one_large_file_can_use_all_connection_permits() {
    let dir = work_dir("chunk-concurrency");
    let body = vec![0x5au8; 1024 * 1024];
    let server = TestServer::start(
        HashMap::from([("/big".to_string(), body.clone())]),
        Duration::from_millis(60),
    )
    .await;

    let item = DownloadItem::new([server.url("/big")], dir.join("big.bin"))
        .with_size(Some(body.len() as u64))
        .with_sha1(Some(hash_bytes(HashAlgorithm::Sha1, &body)));
    let options = DownloadOptions::default()
        .with_chunking(1024, 256 * 1024)
        .with_connection_limit(4)
        .with_max_active_files(4)
        .with_max_workers(2)
        .with_max_retries(0);

    let (_, result, _) = run_items(options, vec![item]).await;
    result.unwrap();

    assert_eq!(std::fs::read(dir.join("big.bin")).unwrap(), body);
    assert_eq!(
        server.max_active(),
        4,
        "单个大文件应当能同时占满 4 个连接许可"
    );
    assert_eq!(server.requests(), 5, "1 次 Range 探测 + 4 个分块");
}

#[tokio::test]
async fn multiple_files_share_the_global_connection_budget() {
    let dir = work_dir("shared-connections");
    let mut files = HashMap::new();
    let mut items = Vec::new();
    for index in 0..3 {
        let path = format!("/file-{index}");
        let body = vec![index as u8; 512 * 1024];
        files.insert(path.clone(), body.clone());
        items.push(
            DownloadItem::new([server_url_placeholder()], dir.join(format!("{index}.bin")))
                .with_size(Some(body.len() as u64))
                .with_sha1(Some(hash_bytes(HashAlgorithm::Sha1, &body))),
        );
    }

    // 先启动服务器，再把占位 URL 换成真实地址。
    let server = TestServer::start(files, Duration::from_millis(40)).await;
    for (index, item) in items.iter_mut().enumerate() {
        item.urls = vec![server.url(&format!("/file-{index}"))];
    }

    let options = DownloadOptions::default()
        .with_chunking(1024, 128 * 1024)
        .with_connection_limit(4)
        .with_max_active_files(3)
        .with_max_workers(2)
        .with_max_retries(0);

    let (_, result, _) = run_items(options, items).await;
    result.unwrap();

    for index in 0..3 {
        let expected = vec![index as u8; 512 * 1024];
        assert_eq!(
            std::fs::read(dir.join(format!("{index}.bin"))).unwrap(),
            expected
        );
    }
    assert!(
        server.max_active() <= 4,
        "全局 in-flight chunk 数不能超过 max_connections"
    );
}

fn server_url_placeholder() -> String {
    "http://127.0.0.1:1/unused".to_string()
}

#[tokio::test]
async fn precheck_skips_complete_files_and_does_not_count_them_in_total() {
    let dir = work_dir("precheck");
    tokio::fs::create_dir_all(&dir).await.unwrap();
    let existing_body = b"already complete".to_vec();
    let missing_body = vec![7u8; 2000];
    let existing = dir.join("existing.bin");
    tokio::fs::write(&existing, &existing_body).await.unwrap();

    let server = TestServer::start(
        HashMap::from([("/missing".to_string(), missing_body.clone())]),
        Duration::ZERO,
    )
    .await;

    let items = vec![
        DownloadItem::new([server.url("/existing")], &existing)
            .with_size(Some(existing_body.len() as u64))
            .with_sha1(Some(hash_bytes(HashAlgorithm::Sha1, &existing_body))),
        DownloadItem::new([server.url("/missing")], dir.join("missing.bin"))
            .with_size(Some(missing_body.len() as u64))
            .with_sha1(Some(hash_bytes(HashAlgorithm::Sha1, &missing_body))),
    ];
    let options = DownloadOptions::default()
        .without_chunking()
        .with_connection_limit(2)
        .with_max_active_files(2)
        .with_max_workers(2)
        .with_max_retries(0);

    let (progress, result, events) = run_items(options, items).await;
    result.unwrap();

    let snapshot = progress.snapshot();
    assert_eq!(
        snapshot.total,
        missing_body.len() as u64,
        "已完成文件不应计入 total"
    );
    assert_eq!(snapshot.downloaded, missing_body.len() as u64);
    assert_eq!(server.requests(), 1, "预检应跳过已完整的文件");

    assert!(
        !events
            .iter()
            .any(|event| matches!(event, TaskEvent::TaskFailed { .. })),
        "{events:?}"
    );
    let added: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            TaskEvent::TaskAdded { task } => Some(task.name().to_string()),
            _ => None,
        })
        .collect();
    assert_eq!(added.len(), 2, "静态准备节点 + 1 个动态文件任务");
    assert!(
        added.iter().any(|name| name == "下载 missing.bin"),
        "{added:?}"
    );
    assert!(
        !added.iter().any(|name| name == "existing.bin"),
        "{added:?}"
    );
}

#[tokio::test]
async fn one_permit_deadlock_regression_with_checksums_and_retries() {
    let dir = work_dir("deadlock");
    let mut files = HashMap::new();
    let mut items = Vec::new();
    for index in 0..2 {
        let path = format!("/file-{index}");
        let body = vec![index as u8; 64 * 1024];
        files.insert(path.clone(), body.clone());
        items.push((path, body));
    }
    let server = TestServer::start(files, Duration::from_millis(20)).await;
    let items: Vec<_> = items
        .into_iter()
        .enumerate()
        .map(|(index, (path, body))| {
            DownloadItem::new([server.url(&path)], dir.join(format!("{index}.bin")))
                .with_size(Some(body.len() as u64))
                .with_sha1(Some(hash_bytes(HashAlgorithm::Sha1, &body)))
        })
        .collect();

    // 三层都只有 1 个许可：获取顺序必须是 file -> connection -> worker，
    // 并在校验前释放 file，否则这里会死锁。
    let options = DownloadOptions::default()
        .without_chunking()
        .with_connection_limit(1)
        .with_max_active_files(1)
        .with_max_workers(1)
        .with_max_retries(2);

    let run = run_items(options, items);
    let (_, result, _) = tokio::time::timeout(Duration::from_secs(10), run)
        .await
        .expect("获取顺序反转会导致死锁");
    result.unwrap();
}

#[tokio::test]
async fn a_failed_file_fails_the_graph() {
    let dir = work_dir("failure");
    let server = TestServer::start(
        HashMap::from([("/good".to_string(), vec![1u8; 128])]),
        Duration::ZERO,
    )
    .await;

    let items = vec![
        DownloadItem::new([server.url("/good")], dir.join("good.bin")).with_size(Some(128)),
        DownloadItem::new([server.url("/missing")], dir.join("missing.bin")),
    ];
    let options = DownloadOptions::default()
        .without_chunking()
        .with_max_retries(0);

    let (_, result, events) = run_items(options, items).await;
    let error = result.unwrap_err();
    assert!(format!("{error:#}").contains("missing"), "{error:#}");
    assert!(!dir.join("missing.bin").exists());
    assert!(!dir.join("missing.bin.part").exists());
    assert!(
        events.iter().any(
            |event| matches!(event, TaskEvent::TaskFailed { task } if task.name() == "下载 missing.bin")
        ),
        "{events:?}"
    );
    assert!(matches!(events.last(), Some(TaskEvent::GraphFailed)));
}

#[tokio::test]
async fn cancelling_releases_semaphore_permits() {
    let dir = work_dir("cancel");
    let slow = TestServer::start(
        HashMap::from([("/slow".to_string(), vec![9u8; 1024])]),
        Duration::from_secs(5),
    )
    .await;
    let fast = TestServer::start(
        HashMap::from([("/fast".to_string(), vec![3u8; 128])]),
        Duration::ZERO,
    )
    .await;

    let options = DownloadOptions::default()
        .without_chunking()
        .with_connection_limit(1)
        .with_max_active_files(1)
        .with_max_workers(2);
    let downloads = downloads(options, ProgressHandle::new());

    let mut builder = TaskGraphBuilder::new("慢下载");
    let step = builder.step("下载");
    let sink = builder.add(
        step,
        "慢文件",
        (),
        downloads.task(DownloadItem::new([slow.url("/slow")], dir.join("slow.bin"))),
    );
    let graph = builder.build(sink).unwrap();
    let cancel = graph.cancel_handle();
    let handle = tokio::spawn(async move { graph.run(|_| {}).await });

    tokio::time::sleep(Duration::from_millis(150)).await;
    cancel.cancel();
    tokio::time::timeout(Duration::from_secs(3), handle)
        .await
        .expect("取消应当立刻结束等待中的下载")
        .unwrap()
        .unwrap_err();

    // 复用同一个上下文：如果许可泄漏，第二次下载会卡在信号量上。
    let mut builder = TaskGraphBuilder::new("快下载");
    let step = builder.step("下载");
    let sink = builder.add(
        step,
        "快文件",
        (),
        downloads.task(DownloadItem::new([fast.url("/fast")], dir.join("fast.bin"))),
    );
    let graph = builder.build(sink).unwrap();
    tokio::time::timeout(Duration::from_secs(3), graph.run(|_| {}))
        .await
        .expect("许可泄漏会让第二次下载卡住")
        .unwrap();

    assert_eq!(std::fs::read(dir.join("fast.bin")).unwrap(), vec![3u8; 128]);
}

#[tokio::test]
async fn tasks_blocked_on_a_semaphore_are_marked_failed_when_cancelled() {
    let dir = work_dir("cancel-waiting");
    let server = TestServer::start(
        HashMap::from([("/f".to_string(), vec![1u8; 64])]),
        Duration::from_secs(5),
    )
    .await;

    // file = 1，第一个任务占住文件许可并等待远端；第二个任务卡在 file 信号量上。
    let options = DownloadOptions::default()
        .without_chunking()
        .with_connection_limit(1)
        .with_max_active_files(1)
        .with_max_workers(1);
    let downloads = downloads(options, ProgressHandle::new());

    let mut builder = TaskGraphBuilder::new("取消等待");
    let step = builder.step("下载");
    let slow_item = DownloadItem::new([server.url("/f")], dir.join("a.bin"));
    let waiting_item = DownloadItem::new([server.url("/f")], dir.join("b.bin"));
    let slow = builder.add(step, "慢", (), downloads.task(slow_item));
    let waiting = builder.add(step, "等待许可", (), downloads.task(waiting_item));
    let sink = builder.add(
        step,
        "结束",
        (slow, waiting),
        task_fn(|_values: (Arc<()>, Arc<()>), _ctx| async move { Ok(()) }),
    );
    let graph = builder.build(sink).unwrap();
    let cancel = graph.cancel_handle();
    let handle = tokio::spawn(async move { graph.run(|_| {}).await });
    tokio::time::sleep(Duration::from_millis(100)).await;
    cancel.cancel();

    let error = tokio::time::timeout(Duration::from_secs(3), handle)
        .await
        .expect("卡在信号量上的任务必须立刻退出")
        .unwrap()
        .unwrap_err();
    assert!(format!("{error:#}").contains("取消"), "{error:#}");
}
