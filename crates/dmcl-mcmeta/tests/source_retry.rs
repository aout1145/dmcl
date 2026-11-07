//! 本地 HTTP 测试服务器：`DownloadSource::fetch_meta` 的退避与限流处理。

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use dmcl_mcmeta::source::{DownloadSource, Mirror};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

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
            let responses = responses.clone();
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

    fn url(&self) -> String {
        format!("http://{}/meta.json", self.addr)
    }

    fn requests(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }
}

async fn read_request(stream: &mut TcpStream) -> Option<()> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        let read = match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => return None,
            Ok(read) => read,
        };
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.windows(4).any(|window| window == b"\r\n\r\n") {
            return Some(());
        }
        if buffer.len() > 64 * 1024 {
            return None;
        }
    }
}

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

fn source(base: Duration, max: Duration, retries: u32) -> DownloadSource {
    DownloadSource::with_priority([Mirror::Official])
        .unwrap()
        .with_max_retries(retries)
        .with_retry_delay(base, max)
}

#[tokio::test]
async fn a_429_response_honours_retry_after() {
    let server = ScriptedServer::start(vec![
        http_response("429 Too Many Requests", &[("Retry-After", "1")], b""),
        http_response("200 OK", &[], b"{}"),
    ])
    .await;

    // 指数退避故意设得比 Retry-After 短：确认等的是响应头。
    let source = source(Duration::from_millis(10), Duration::from_millis(10), 1);
    let started = Instant::now();
    assert_eq!(source.fetch_meta(&server.url()).await.unwrap(), "{}");
    assert!(
        started.elapsed() >= Duration::from_millis(900),
        "应当等待 Retry-After 给出的 1 秒"
    );
    assert_eq!(server.requests(), 2);
}

#[tokio::test]
async fn bmclapi_rate_limit_header_is_used_when_retry_after_is_missing() {
    let server = ScriptedServer::start(vec![
        http_response(
            "429 Too Many Requests",
            &[("x-ratelimit-reset-short", "0")],
            b"",
        ),
        http_response("200 OK", &[], b"{}"),
    ])
    .await;

    // 如果没读到 bmclapi 的限流头，就会退避 5 秒。
    let source = source(Duration::from_secs(5), Duration::from_secs(5), 1);
    let started = Instant::now();
    assert_eq!(source.fetch_meta(&server.url()).await.unwrap(), "{}");
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "x-ratelimit-reset-short 应当覆盖指数退避"
    );
    assert_eq!(server.requests(), 2);
}

#[tokio::test]
async fn transient_errors_back_off_exponentially() {
    let server =
        ScriptedServer::start(vec![http_response("500 Internal Server Error", &[], b"")]).await;

    let source = source(Duration::from_millis(100), Duration::from_secs(10), 2);
    let started = Instant::now();
    source.fetch_meta(&server.url()).await.unwrap_err();
    // 第 1 次重试等 100ms，第 2 次等 200ms。
    assert!(
        started.elapsed() >= Duration::from_millis(280),
        "两次重试必须按指数退避等待"
    );
    assert_eq!(server.requests(), 3);
}
