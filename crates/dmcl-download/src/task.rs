//! 下载任务本体。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
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
/// 建立连接的时限（TCP / TLS）。
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// 两次读之间的空闲时限：超过它就认为连接僵死，报错并交给重试。
const READ_TIMEOUT: Duration = Duration::from_secs(30);

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
            .connect_timeout(CONNECT_TIMEOUT)
            .read_timeout(READ_TIMEOUT)
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

    /// 已知大小的文件的总字节数。
    pub fn known_total_bytes(&self) -> u64 {
        self.items.iter().filter_map(|item| item.size).sum()
    }

    /// 执行下载并返回完整报告。
    ///
    /// 单个文件失败**不会**让这里返回 `Err`（唯一例外是被取消）——失败项都在报告里。
    /// 任务链里的语义由 [`Task`] 的实现给出（默认"有失败就报错"）。
    pub async fn execute(&self, ctx: &TaskContext) -> Result<DownloadReport> {
        let options = Arc::new(self.options.normalized());
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

        // 进度分母只统计「真正要下载、且知道大小」的文件 —— 跳过项不计入，
        // 未知大小的项也不计入，重试期间会被回滚，因此 `done` 永远不会超过 `total`
        let known_total = pending.iter().filter_map(|item| item.size).sum::<u64>();
        let tracker = Arc::new(Tracker::new(
            if known_total > 0 {
                Some(known_total)
            } else {
                None
            },
            pending.len(),
        ));
        let connections = Arc::new(Semaphore::new(options.max_connections));

        if !pending.is_empty() {
            ctx.status(format!("待下载 {} 个文件", pending.len()));
        }

        let mut set: JoinSet<ItemOutcome> = JoinSet::new();
        for item in pending {
            let options = options.clone();
            let tracker = tracker.clone();
            let connections = connections.clone();
            let client = self.client.clone();
            let ctx = ctx.clone();
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
            // 不管成败，这个文件都算"处理完一个"
            tracker.file_finished(ctx);
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
            // 每次尝试单独记账：失败重试时把这一轮已经上报的字节数减回去，
            // 否则进度会变成"100% 了还在涨"。
            let recorder =
                Recorder::new(self.tracker.clone(), self.ctx.clone(), item.size.is_some());
            match self.attempt(&item, &save_to, &recorder).await {
                Ok(bytes) => {
                    recorder.commit();
                    return ItemOutcome {
                        item,
                        result: Ok(bytes),
                    };
                }
                Err(err) => {
                    recorder.discard();
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
    async fn attempt(
        &self,
        item: &DownloadItem,
        save_to: &Path,
        recorder: &Recorder,
    ) -> Result<u64> {
        if let Some(parent) = save_to.parent() {
            tokio::fs::create_dir_all(parent).await.ok();
        }
        let part = item.part_path();
        let _ = tokio::fs::remove_file(&part).await;

        // 已知大小且足够大 → 先探测服务器是否支持 Range，再决定分块还是单连接
        let chunked_total = match item.size {
            Some(size) if self.options.should_chunk(size) => {
                probe_range(self.client, &item.url).await
            }
            _ => None,
        };

        let bytes = match chunked_total {
            Some(total) => self.chunked(item, &part, total, recorder).await?,
            None => self.single(item, &part, recorder).await?,
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
    async fn single(&self, item: &DownloadItem, part: &Path, recorder: &Recorder) -> Result<u64> {
        let _permit = self.connections.acquire().await?;
        let mut response = send_get(self.client, &item.url, None)
            .await?
            .error_for_status()?;
        let mut file = tokio::fs::File::create(part).await?;
        let mut written = 0u64;
        while let Some(chunk) = response.chunk().await? {
            recorder.check_cancelled()?;
            file.write_all(&chunk).await?;
            written += chunk.len() as u64;
            recorder.add(chunk.len() as u64);
        }
        file.flush().await?;
        Ok(written)
    }

    /// 分块并发下载。
    async fn chunked(
        &self,
        item: &DownloadItem,
        part: &Path,
        size: u64,
        recorder: &Recorder,
    ) -> Result<u64> {
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
            let recorder = recorder.clone();
            let url = item.url.clone();
            let part = part.to_path_buf();
            set.spawn(async move {
                // 先占"单文件块数"名额，再占全局连接名额
                let _file_permit = file_semaphore.acquire_owned().await?;
                let _permit = connections.acquire_owned().await?;
                fetch_range(&client, &url, start, end, &part, &recorder).await
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
    recorder: &Recorder,
) -> Result<u64> {
    let range = format!("bytes={start}-{end}");
    let mut response = send_get(client, url, Some(range))
        .await?
        .error_for_status()?;
    if response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
        anyhow::bail!("服务器没有按 Range 请求返回 206（{}）", response.status());
    }

    let mut file = tokio::fs::OpenOptions::new().write(true).open(part).await?;
    file.seek(std::io::SeekFrom::Start(start)).await?;

    let mut written = 0u64;
    while let Some(chunk) = response.chunk().await? {
        recorder.check_cancelled()?;
        file.write_all(&chunk).await?;
        written += chunk.len() as u64;
        recorder.add(chunk.len() as u64);
    }
    file.flush().await?;

    let expected = end - start + 1;
    if written != expected {
        anyhow::bail!("分块 {start}-{end} 长度不符（期望 {expected}，实际 {written}）");
    }
    Ok(written)
}

/// 发一个 GET 请求，等待响应头也有时限（防止服务器 accept 之后僵住不回包）。
async fn send_get(
    client: &reqwest::Client,
    url: &str,
    range: Option<String>,
) -> Result<reqwest::Response> {
    let mut request = client.get(url);
    if let Some(range) = range {
        request = request.header(reqwest::header::RANGE, range);
    }
    tokio::time::timeout(READ_TIMEOUT, request.send())
        .await
        .map_err(|_| anyhow!("等待服务器响应超时（{} 秒）", READ_TIMEOUT.as_secs()))?
        .map_err(Into::into)
}

/// 探测服务器是否支持 Range，返回资源总大小。
///
/// 只发一个 `bytes=0-0` 的请求；不支持（不是 206、或没有 `Content-Range`）时返回
/// `None`，调用者就退回单连接下载。
async fn probe_range(client: &reqwest::Client, url: &str) -> Option<u64> {
    let fut = client
        .get(url)
        .header(reqwest::header::RANGE, "bytes=0-0")
        .send();
    let response = tokio::time::timeout(READ_TIMEOUT, fut).await.ok()?.ok()?;
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

// ---------------------------------------------------------------------------
// 进度记账
// ---------------------------------------------------------------------------

/// 一次任务运行里所有文件共享的进度汇总。
///
/// 与旧实现的关键区别：它用**步骤上下文**（而不是某个文件的名字）上报，因此
/// 进度行会显示"下载库文件 41.2% (…/…) 35/120 个文件"，而不是把全局数字
/// 挂到某个恰好触发上报的文件名下。
struct Tracker {
    done: AtomicU64,
    total: Option<u64>,
    files_done: AtomicUsize,
    files_total: usize,
    last: Mutex<Instant>,
}

impl Tracker {
    fn new(total: Option<u64>, files_total: usize) -> Self {
        Self {
            done: AtomicU64::new(0),
            total,
            files_done: AtomicUsize::new(0),
            files_total,
            last: Mutex::new(Instant::now()),
        }
    }

    /// 累计下载字节。只对"知道大小"的文件调用（`count_bytes` 由调用者把关）。
    fn add(&self, bytes: u64, ctx: &TaskContext) {
        self.done.fetch_add(bytes, Ordering::Relaxed);
        self.maybe_report(ctx);
    }

    /// 重试失败时把这一轮已经上报的字节减回去。
    fn rollback(&self, bytes: u64) {
        self.done.fetch_sub(bytes, Ordering::Relaxed);
    }

    /// 一个文件处理完（成功或失败）。
    fn file_finished(&self, ctx: &TaskContext) {
        self.files_done.fetch_add(1, Ordering::Relaxed);
        self.maybe_report(ctx);
    }

    fn message(&self) -> String {
        format!(
            "{}/{} 个文件",
            self.files_done.load(Ordering::Relaxed),
            self.files_total
        )
    }

    fn maybe_report(&self, ctx: &TaskContext) {
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
            let done = self.done.load(Ordering::Relaxed);
            ctx.progress(done, self.total, self.message());
        }
    }

    /// 收尾时强制上报一次，并给出汇总。
    fn finish(&self, ctx: &TaskContext, report: &DownloadReport) {
        if self.files_total > 0 {
            let done = self.done.load(Ordering::Relaxed);
            ctx.progress(done, self.total, self.message());
        }
        ctx.status(format!(
            "下载完成：新下载 {} 个，跳过 {} 个，失败 {} 个（{}）",
            report.downloaded,
            report.skipped,
            report.failed.len(),
            human_bytes(report.bytes)
        ));
    }
}

/// 一次下载尝试的字节记账：既累计进整体进度，也记下这一轮写了多少，
/// 失败重试时好把那部分减回去（[`Recorder::discard`]）。
#[derive(Clone)]
struct Recorder {
    tracker: Arc<Tracker>,
    ctx: TaskContext,
    /// 这一项知道大小时才把字节计入分母（未知大小的项不该影响百分比）。
    count_bytes: bool,
    attempted: Arc<AtomicU64>,
}

impl Recorder {
    fn new(tracker: Arc<Tracker>, ctx: TaskContext, count_bytes: bool) -> Self {
        Self {
            tracker,
            ctx,
            count_bytes,
            attempted: Arc::new(AtomicU64::new(0)),
        }
    }

    fn add(&self, bytes: u64) {
        self.attempted.fetch_add(bytes, Ordering::Relaxed);
        if self.count_bytes {
            self.tracker.add(bytes, &self.ctx);
        }
    }

    /// 成功：这一轮的字节保留在整体进度里。
    fn commit(&self) {
        self.attempted.store(0, Ordering::Relaxed);
    }

    /// 失败：把这一轮上报过的字节从整体进度里抹掉。
    fn discard(&self) {
        let bytes = self.attempted.swap(0, Ordering::Relaxed);
        if self.count_bytes {
            self.tracker.rollback(bytes);
        }
    }

    fn check_cancelled(&self) -> Result<()> {
        self.ctx.check_cancelled()
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

    #[test]
    fn the_client_is_built_with_timeouts() {
        // 超时是在客户端构建时写死的；这里只验证构造不会 panic 且拿到默认选项
        let task = DownloadTask::new(DownloadOptions::default());
        assert_eq!(task.options().max_attempts, 3);
        let _ = task.client;
    }

    /// 重试时进度要回滚：一次失败尝试把已上报的字节减回去，`done` 不会超过 `total`。
    #[test]
    fn a_recorder_rolls_back_a_discarded_attempt() {
        let ctx = TaskContext::detached("下载");
        let tracker = Arc::new(Tracker::new(Some(100), 1));
        let recorder = Recorder::new(tracker.clone(), ctx, true);

        recorder.add(40);
        recorder.add(60);
        assert_eq!(tracker.done.load(Ordering::Relaxed), 100);

        // 这次尝试作废，字节要抹掉
        recorder.discard();
        assert_eq!(tracker.done.load(Ordering::Relaxed), 0);

        // 成功的尝试则保留
        let recorder = Recorder::new(tracker.clone(), TaskContext::detached("下载"), true);
        recorder.add(50);
        recorder.commit();
        assert_eq!(tracker.done.load(Ordering::Relaxed), 50);
    }

    /// 不知道大小的文件不影响字节进度（只推进"文件个数"）。
    #[test]
    fn unknown_sized_files_do_not_touch_the_byte_counter() {
        let ctx = TaskContext::detached("下载");
        let tracker = Arc::new(Tracker::new(Some(100), 2));
        let recorder = Recorder::new(tracker.clone(), ctx.clone(), false);

        recorder.add(999);
        assert_eq!(tracker.done.load(Ordering::Relaxed), 0);

        tracker.file_finished(&ctx);
        assert_eq!(tracker.message(), "1/2 个文件");
    }
}
