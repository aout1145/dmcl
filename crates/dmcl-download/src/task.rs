//! 下载任务本体。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use path_clean::PathClean;
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use dmcl_task::{human_bytes, BoxFuture, Cancelled, Task, TaskContext};

use crate::hash::{file_matches, hash_file};
use crate::item::DownloadItem;
use crate::options::DownloadOptions;
use crate::report::{DownloadFailure, DownloadReport};
/// 进度事件的节流间隔（避免每 64 KiB 就刷一次事件）。
const REPORT_INTERVAL: Duration = Duration::from_millis(100);

/// 把一组文件下载到本地。
///
/// 见 crate 文档了解并发 / 分块 / `.part` 策略。
#[derive(Debug)]
pub struct DownloadTask {
    name: String,
    items: Vec<DownloadItem>,
    options: DownloadOptions,
    client: reqwest::Client,
}

impl DownloadTask {
    pub fn new(options: DownloadOptions) -> Self {
        let options = options.normalized();
        let client = reqwest::Client::builder()
            .user_agent(options.user_agent.clone())
            .build()
            .unwrap_or_default();
        Self {
            name: "下载".to_string(),
            items: Vec::new(),
            options,
            client,
        }
    }

    /// 任务名（进度事件里的名字）。
    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// 用外部提供的 http 客户端（例如复用一个连接池）。
    pub fn with_client(mut self, client: reqwest::Client) -> Self {
        self.client = client;
        self
    }

    pub fn push(&mut self, item: DownloadItem) -> &mut Self {
        self.items.push(item);
        self
    }

    pub fn extend(&mut self, items: impl IntoIterator<Item = DownloadItem>) -> &mut Self {
        self.items.extend(items);
        self
    }

    /// [`DownloadTask::push`] 的链式版本。
    pub fn with_item(mut self, item: DownloadItem) -> Self {
        self.items.push(item);
        self
    }

    pub fn items(&self) -> &[DownloadItem] {
        &self.items
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn options(&self) -> &DownloadOptions {
        &self.options
    }

    /// 已知大小的文件的总字节数（进度条的分母）。
    pub fn known_total_bytes(&self) -> u64 {
        self.items.iter().filter_map(|item| item.size).sum()
    }

    /// 执行下载并返回完整报告。
    ///
    /// 单个文件失败**不会**让这里返回 `Err`（唯一例外是被取消）——失败项都在报告里。
    /// 任务链里的语义由 [`Task`] 的实现给出（默认"有失败就报错"）。
    pub async fn execute(&self, ctx: &TaskContext) -> Result<DownloadReport> {
        let options = Arc::new(self.options.normalized());
        let known_total = self.known_total_bytes();
        let tracker = Arc::new(Tracker::new(if known_total > 0 {
            Some(known_total)
        } else {
            None
        }));
        let connections = Arc::new(Semaphore::new(options.max_connections));
        let report = Arc::new(Mutex::new(DownloadReport::default()));

        // 先做本地判断：已经是最新（校验值 / 大小一致）的文件不占用连接
        let mut pending: Vec<DownloadItem> = Vec::new();
        for item in &self.items {
            let save_to = absolute(&item.save_to);
            if file_matches(&save_to, item.checksum.as_ref(), item.size).await {
                report.lock().unwrap().skipped += 1;
            } else {
                pending.push(item.clone());
            }
        }
        if !pending.is_empty() {
            ctx.status(format!("待下载 {} 个文件", pending.len()));
        }

        let mut set: JoinSet<ItemOutcome> = JoinSet::new();
        for item in pending {
            let ctx = ctx.child(item.name.as_str());
            let options = options.clone();
            let tracker = tracker.clone();
            let connections = connections.clone();
            let client = self.client.clone();
            set.spawn(async move {
                download_one(&client, &options, &tracker, &connections, item, &ctx).await
            });
        }

        let mut cancelled = false;
        while let Some(joined) = set.join_next().await {
            let outcome = match joined {
                Ok(outcome) => outcome,
                Err(join_error) => {
                    report.lock().unwrap().failed.push(DownloadFailure {
                        path: PathBuf::new(),
                        url: String::new(),
                        error: format!("下载任务异常终止: {join_error}"),
                    });
                    continue;
                }
            };
            match outcome.result {
                Ok(bytes) => {
                    let mut report = report.lock().unwrap();
                    report.downloaded += 1;
                    report.bytes += bytes;
                }
                Err(err) if err.downcast_ref::<Cancelled>().is_some() => {
                    cancelled = true;
                    set.abort_all();
                }
                Err(err) => {
                    report.lock().unwrap().failed.push(DownloadFailure {
                        path: outcome.item.save_to.clone(),
                        url: outcome.item.url.clone(),
                        error: format!("{err:#}"),
                    });
                }
            }
        }

        if cancelled {
            return Err(anyhow::Error::new(Cancelled));
        }

        let report = std::mem::take(&mut *report.lock().unwrap());
        tracker.finish(ctx, &report);
        Ok(report)
    }
}

impl Task for DownloadTask {
    fn name(&self) -> &str {
        &self.name
    }

    fn run<'a>(&'a self, ctx: TaskContext) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let report = self.execute(&ctx).await?;
            if report.failed.is_empty() || self.options.ignore_failures {
                Ok(())
            } else {
                Err(anyhow!(
                    "有 {} 个文件下载失败：\n{}",
                    report.failed.len(),
                    report.failure_summary(5)
                ))
            }
        })
    }
}

struct ItemOutcome {
    item: DownloadItem,
    result: Result<u64>,
}

/// 一次任务运行里所有文件共享的东西。
struct Downloader<'a> {
    client: &'a reqwest::Client,
    options: &'a DownloadOptions,
    tracker: &'a Arc<Tracker>,
    connections: &'a Arc<Semaphore>,
    ctx: &'a TaskContext,
}

impl Downloader<'_> {
    /// 一个文件的完整流程（含重试）。
    async fn download_one(&self, item: DownloadItem) -> ItemOutcome {
        let save_to = absolute(&item.save_to);
        let mut last_error: Option<anyhow::Error> = None;

        for attempt in 1..=self.options.max_attempts {
            if let Err(err) = self.ctx.check_cancelled() {
                return ItemOutcome {
                    item,
                    result: Err(err),
                };
            }
            if attempt > 1 {
                self.ctx.status(format!(
                    "重试 {}/{}: {}",
                    attempt, self.options.max_attempts, item.name
                ));
            }
            match self.attempt(&item, &save_to).await {
                Ok(bytes) => {
                    return ItemOutcome {
                        item,
                        result: Ok(bytes),
                    }
                }
                Err(err) => {
                    // 失败后清掉半成品，下一轮从头来过
                    let _ = tokio::fs::remove_file(item.part_path()).await;
                    last_error = Some(err.context(format!(
                        "第 {attempt}/{} 次下载 {} 失败",
                        self.options.max_attempts, item.url
                    )));
                }
            }
        }

        ItemOutcome {
            item,
            result: Err(last_error.unwrap_or_else(|| anyhow!("下载失败"))),
        }
    }

    /// 一次尝试：下载 → 校验 → 改名。失败时不会留下最终文件。
    async fn attempt(&self, item: &DownloadItem, save_to: &Path) -> Result<u64> {
        if let Some(parent) = save_to.parent() {
            tokio::fs::create_dir_all(parent).await.ok();
        }
        let part = item.part_path();
        let _ = tokio::fs::remove_file(&part).await;
        self.ctx.status(format!("开始下载 {}", item.name));

        // 已知大小且足够大 → 先探测服务器是否支持 Range，再决定分块还是单连接
        let chunked_total = match item.size {
            Some(size) if self.options.should_chunk(size) => {
                probe_range(self.client, &item.url).await
            }
            _ => None,
        };

        let bytes = match chunked_total {
            Some(total) => self.chunked(item, &part, total).await?,
            None => self.single(item, &part).await?,
        };

        // 校验：校验值优先（用哪个算法由它决定），其次大小
        if let Some(checksum) = item.checksum.as_ref() {
            let actual = hash_file(&part, checksum.algorithm()).await?;
            if actual != checksum.hex() {
                anyhow::bail!(
                    "{} 校验失败（期望 {}，实际 {actual}）",
                    checksum.algorithm(),
                    checksum.hex()
                );
            }
        } else if let Some(size) = item.size {
            if bytes != size {
                anyhow::bail!("大小不匹配（期望 {size} 字节，实际 {bytes} 字节）");
            }
        }

        tokio::fs::rename(&part, save_to)
            .await
            .with_context(|| format!("无法把 {} 改名为 {}", part.display(), save_to.display()))?;
        Ok(bytes)
    }

    /// 单连接下载。
    async fn single(&self, item: &DownloadItem, part: &Path) -> Result<u64> {
        let _permit = self.connections.acquire().await?;
        let mut response = self
            .client
            .get(&item.url)
            .send()
            .await?
            .error_for_status()?;
        let mut file = tokio::fs::File::create(part).await?;
        let mut written = 0u64;
        while let Some(chunk) = response.chunk().await? {
            self.ctx.check_cancelled()?;
            file.write_all(&chunk).await?;
            written += chunk.len() as u64;
            self.tracker.add(chunk.len() as u64, self.ctx);
        }
        file.flush().await?;
        Ok(written)
    }

    /// 分块并发下载。
    async fn chunked(&self, item: &DownloadItem, part: &Path, size: u64) -> Result<u64> {
        // 预分配出目标大小：各分块各自 seek 到自己的偏移写入，互不干扰
        {
            let file = tokio::fs::File::create(part).await?;
            file.set_len(size).await?;
        }

        let file_semaphore = Arc::new(Semaphore::new(self.options.max_chunks_per_file));
        let chunk_size = self.options.chunk_size;
        let mut set: JoinSet<Result<u64>> = JoinSet::new();

        let mut start = 0u64;
        while start < size {
            let end = (start + chunk_size - 1).min(size - 1);
            let client = self.client.clone();
            let connections = self.connections.clone();
            let file_semaphore = file_semaphore.clone();
            let tracker = self.tracker.clone();
            let ctx = self.ctx.clone();
            let url = item.url.clone();
            let part = part.to_path_buf();
            set.spawn(async move {
                // 先占"单文件块数"名额，再占全局连接名额
                let _file_permit = file_semaphore.acquire_owned().await?;
                let _permit = connections.acquire_owned().await?;
                fetch_range(&client, &url, start, end, &part, &tracker, &ctx).await
            });
            start = end + 1;
        }

        let mut written = 0u64;
        while let Some(joined) = set.join_next().await {
            match joined {
                Ok(Ok(bytes)) => written += bytes,
                Ok(Err(err)) => {
                    set.abort_all();
                    return Err(err);
                }
                Err(join_error) => {
                    set.abort_all();
                    return Err(anyhow!("分块下载任务异常终止: {join_error}"));
                }
            }
        }
        Ok(written)
    }
}

/// 一个文件的重试循环。
async fn download_one(
    client: &reqwest::Client,
    options: &Arc<DownloadOptions>,
    tracker: &Arc<Tracker>,
    connections: &Arc<Semaphore>,
    item: DownloadItem,
    ctx: &TaskContext,
) -> ItemOutcome {
    let downloader = Downloader {
        client,
        options,
        tracker,
        connections,
        ctx,
    };
    downloader.download_one(item).await
}

/// 抓取 `[start, end]` 区间的字节并写到文件的对应偏移。
async fn fetch_range(
    client: &reqwest::Client,
    url: &str,
    start: u64,
    end: u64,
    part: &Path,
    tracker: &Arc<Tracker>,
    ctx: &TaskContext,
) -> Result<u64> {
    let mut response = client
        .get(url)
        .header(reqwest::header::RANGE, format!("bytes={start}-{end}"))
        .send()
        .await?
        .error_for_status()?;
    if response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
        anyhow::bail!("服务器没有按 Range 请求返回 206（{}）", response.status());
    }

    let mut file = tokio::fs::OpenOptions::new().write(true).open(part).await?;
    file.seek(std::io::SeekFrom::Start(start)).await?;

    let mut written = 0u64;
    while let Some(chunk) = response.chunk().await? {
        ctx.check_cancelled()?;
        file.write_all(&chunk).await?;
        written += chunk.len() as u64;
        tracker.add(chunk.len() as u64, ctx);
    }
    file.flush().await?;

    let expected = end - start + 1;
    if written != expected {
        anyhow::bail!("分块 {start}-{end} 长度不符（期望 {expected}，实际 {written}）");
    }
    Ok(written)
}

/// 探测服务器是否支持 Range，返回资源总大小。
///
/// 只发一个 `bytes=0-0` 的请求；不支持（不是 206、或没有 `Content-Range`）时返回
/// `None`，调用者就退回单连接下载。
async fn probe_range(client: &reqwest::Client, url: &str) -> Option<u64> {
    let response = client
        .get(url)
        .header(reqwest::header::RANGE, "bytes=0-0")
        .send()
        .await
        .ok()?;
    if response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
        return None;
    }
    let header = response
        .headers()
        .get(reqwest::header::CONTENT_RANGE)?
        .to_str()
        .ok()?;
    parse_content_range(header)
}

/// 从 `bytes 0-0/1234` 里取出总长度。
fn parse_content_range(value: &str) -> Option<u64> {
    value.rsplit('/').next()?.trim().parse().ok()
}

/// 进度汇总：跨文件累计已完成的字节数，并按 [`REPORT_INTERVAL`] 节流上报。
struct Tracker {
    done: AtomicU64,
    total: Option<u64>,
    last: Mutex<Instant>,
}

impl Tracker {
    fn new(total: Option<u64>) -> Self {
        Self {
            done: AtomicU64::new(0),
            total,
            last: Mutex::new(Instant::now()),
        }
    }

    fn add(&self, bytes: u64, ctx: &TaskContext) {
        let done = self.done.fetch_add(bytes, Ordering::Relaxed) + bytes;
        let due = match self.last.lock() {
            Ok(mut last) => {
                if last.elapsed() >= REPORT_INTERVAL {
                    *last = Instant::now();
                    true
                } else {
                    false
                }
            }
            Err(_) => false,
        };
        if due {
            ctx.progress(done, self.total, ctx.name());
        }
    }

    /// 收尾时强制上报一次，让进度条走到 100%。
    fn finish(&self, ctx: &TaskContext, report: &DownloadReport) {
        let done = self.done.load(Ordering::Relaxed);
        ctx.progress(done, self.total, ctx.name());
        ctx.status(format!(
            "下载完成：新下载 {} 个，跳过 {} 个，失败 {} 个（{}）",
            report.downloaded,
            report.skipped,
            report.failed.len(),
            human_bytes(report.bytes)
        ));
    }
}

/// 让目标路径变成绝对路径并归一化。
fn absolute(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    absolute.clean()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_range_headers_are_parsed() {
        assert_eq!(parse_content_range("bytes 0-0/1234"), Some(1234));
        assert_eq!(parse_content_range("bytes 0-1023/2048"), Some(2048));
        assert_eq!(parse_content_range("garbage"), None);
        assert_eq!(parse_content_range("bytes 0-0/*"), None);
    }

    #[test]
    fn paths_are_normalised_like_everywhere_else() {
        let path = absolute(Path::new("a/./b/../c"));
        assert!(path.is_absolute());
        assert!(path.ends_with("a/c"), "{path:?}");
    }

    #[test]
    fn chunk_ranges_cover_the_file_exactly() {
        // 与 `chunked` 里的循环保持一致：分块必须刚好覆盖 [0, size)
        let size = 10_000u64;
        let chunk_size = 4096u64;
        let mut ranges = Vec::new();
        let mut start = 0u64;
        while start < size {
            let end = (start + chunk_size - 1).min(size - 1);
            ranges.push((start, end));
            start = end + 1;
        }
        assert_eq!(ranges, vec![(0, 4095), (4096, 8191), (8192, 9999)]);
        assert_eq!(ranges.last().unwrap().1, size - 1);
        let covered: u64 = ranges.iter().map(|(start, end)| end - start + 1).sum();
        assert_eq!(covered, size);
    }

    #[test]
    fn the_task_reports_its_items() {
        let mut task = DownloadTask::new(DownloadOptions::default())
            .named("下载游戏")
            .with_item(
                DownloadItem::new("https://example.com/a.jar", "/tmp/a.jar").with_size(Some(10)),
            );
        task.push(DownloadItem::new("https://example.com/b.jar", "/tmp/b.jar"));
        assert_eq!(task.len(), 2);
        assert!(!task.is_empty());
        assert_eq!(task.name(), "下载游戏");
        // 只有知道大小的文件计入分母
        assert_eq!(task.known_total_bytes(), 10);
        assert_eq!(task.items()[0].name, "a.jar");
        assert_eq!(task.options().max_connections, 64);
    }
}
