//! 下载任务本体。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use path_clean::PathClean;
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use dmcl_task::{human_bytes, BoxFuture, Cancelled, Progress, Task, TaskContext};

use crate::hash::{file_ok, hash_file};
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

        // 候选项不能为空：没有候选地址就无从下载，直接报错
        for item in &self.items {
            if item.urls.is_empty() {
                anyhow::bail!("下载项没有候选地址：{}", item.save_to.display());
            }
        }

        // 先做本地判断：已经是最新（校验值 / 大小一致）的文件不占用连接
        let mut pending: Vec<DownloadItem> = Vec::new();
        for item in &self.items {
            let save_to = absolute(&item.save_to);
            if file_ok(&save_to, item.checksum.as_ref(), item.size).await {
                report.lock().unwrap().skipped += 1;
            } else {
                pending.push(item.clone());
            }
        }

        // 进度总量**不预填**：从 0 开始，随每个文件从服务器拿到的实际大小动态累加
        // （见 `Tracker::record_size`）。跳过的项不计入。
        let tracker = Arc::new(Tracker::new(pending.len()));
        let connections = Arc::new(Semaphore::new(options.max_connections));

        if !pending.is_empty() {
            ctx.log(format!("待下载 {} 个文件", pending.len()));
            // 立刻显示文件数量，不用等拿到字节总量
            tracker.initial_report(ctx);
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
                    {
                        let mut report = report.lock().unwrap();
                        report.downloaded += 1;
                        report.bytes += bytes;
                    }
                    ctx.log(format!("{} 完成", outcome.item.name));
                }
                Err(err) if err.downcast_ref::<Cancelled>().is_some() => {
                    cancelled = true;
                    set.abort_all();
                }
                Err(err) => {
                    report.lock().unwrap().failed.push(DownloadFailure {
                        path: outcome.item.save_to.clone(),
                        url: outcome.url.clone(),
                        error: format!("{err:#}"),
                    });
                }
            }
            // 不管成败，这个文件都算"处理完一个"
            tracker.file_finished(outcome.sized, ctx);
        }

        if cancelled {
            return Err(anyhow::Error::new(Cancelled));
        }

        let report = std::mem::take(&mut *report.lock().unwrap());
        tracker.finish(ctx, &report);
        Ok(report)
    }
}

impl Task<(), DownloadReport> for DownloadTask {
    fn name(&self) -> &str {
        &self.name
    }

    fn run<'a>(&'a self, _input: (), ctx: TaskContext) -> BoxFuture<'a, Result<DownloadReport>> {
        Box::pin(async move {
            let report = self.execute(&ctx).await?;
            if report.failed.is_empty() || self.options.ignore_failures {
                Ok(report)
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
    /// 完成时是否已经知道实际大小（决定它算「已知」还是「未知」的完成）。
    sized: bool,
    /// 最后一次尝试用的地址（失败上报用）。
    url: String,
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
    /// 一个文件的完整流程（含重试）。每次尝试**换用下一个候选地址**（A B A …）。
    async fn download_one(&self, item: DownloadItem) -> ItemOutcome {
        let save_to = absolute(&item.save_to);
        let mut last_error: Option<anyhow::Error> = None;
        // 这一项在进度里的份额：是否已知实际大小、记了多少。整项失败时回滚。
        let share = Share::default();
        // 最后尝试过的地址（失败上报用）；还没试过就退回首选
        let mut last_url = item.urls.first().cloned().unwrap_or_default();

        for attempt in 1..=self.options.max_attempts {
            if let Err(err) = self.ctx.check_cancelled() {
                self.tracker.rollback_item(&share);
                return ItemOutcome {
                    item,
                    sized: share.is_sized(),
                    url: last_url,
                    result: Err(err),
                };
            }
            // 轮换候选源：第 n 次尝试用 urls[(n-1) % len]
            let url = item.urls[candidate_index(&item.urls, attempt)].clone();
            last_url = url.clone();
            if attempt > 1 {
                self.ctx.log(format!(
                    "{} 重试（{}/{}）",
                    item.name, attempt, self.options.max_attempts
                ));
            }
            // 每次尝试单独记账：失败重试时把这一轮已经上报的字节数减回去，
            // 否则进度会变成"100% 了还在涨"。
            let recorder =
                Recorder::new(self.tracker.clone(), self.ctx.clone(), share.sized.clone());
            match self.attempt(&item, &url, &save_to, &recorder, &share).await {
                Ok(bytes) => {
                    recorder.commit();
                    return ItemOutcome {
                        item,
                        sized: share.is_sized(),
                        url: last_url,
                        result: Ok(bytes),
                    };
                }
                Err(err) => {
                    recorder.discard();
                    // 失败后清掉半成品，下一轮从头来过
                    let _ = tokio::fs::remove_file(item.part_path()).await;
                    last_error = Some(err.context(format!(
                        "第 {attempt}/{} 次从 {url} 下载 {} 失败",
                        self.options.max_attempts, item.name
                    )));
                }
            }
        }

        // 全部尝试都失败：把这一项从进度里回滚掉
        self.tracker.rollback_item(&share);
        ItemOutcome {
            item,
            sized: share.is_sized(),
            url: last_url,
            result: Err(last_error.unwrap_or_else(|| anyhow!("下载失败"))),
        }
    }

    /// 一次尝试：下载 → 校验 → 改名。失败时不会留下最终文件。
    async fn attempt(
        &self,
        item: &DownloadItem,
        url: &str,
        save_to: &Path,
        recorder: &Recorder,
        share: &Share,
    ) -> Result<u64> {
        if let Some(parent) = save_to.parent() {
            tokio::fs::create_dir_all(parent).await.ok();
        }
        let part = item.part_path();
        let _ = tokio::fs::remove_file(&part).await;

        // 已知大小且足够大 → 先探测服务器是否支持 Range，再决定分块还是单连接
        let chunked_total = match item.size {
            Some(size) if self.options.should_chunk(size) => probe_range(self.client, url).await,
            _ => None,
        };

        let bytes = match chunked_total {
            Some(total) => {
                // 分块的总量来自 `Content-Range`（服务器实际大小）
                self.tracker.record_size(share, total);
                self.chunked(url, &part, total, recorder).await?
            }
            None => self.single(url, &part, recorder, share).await?,
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
    async fn single(
        &self,
        url: &str,
        part: &Path,
        recorder: &Recorder,
        share: &Share,
    ) -> Result<u64> {
        let _permit = self.connections.acquire().await?;
        let mut response = send_get(self.client, url, None).await?.error_for_status()?;
        // 服务器给的实际大小：这一项转入「已知」组
        if let Some(actual) = response.content_length() {
            self.tracker.record_size(share, actual);
        }
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
    async fn chunked(&self, url: &str, part: &Path, size: u64, recorder: &Recorder) -> Result<u64> {
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
            let url = url.to_string();
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

/// 第 `attempt`（从 1 开始）次尝试该用哪个候选地址：按优先级轮换（A B A …）。
fn candidate_index(urls: &[String], attempt: u32) -> usize {
    (attempt.saturating_sub(1) as usize) % urls.len()
}

// ---------------------------------------------------------------------------
// 进度记账
// ---------------------------------------------------------------------------

/// 一个文件在进度里的份额状态：是否已知实际大小、记了多少。
///
/// `sized` 用 `Arc` 是为了让 [`Recorder`]（分块下载时会克隆进各个分块任务）共享
/// 同一个开关。
struct Share {
    sized: Arc<AtomicBool>,
    recorded: AtomicU64,
}

impl Default for Share {
    fn default() -> Self {
        Self {
            sized: Arc::new(AtomicBool::new(false)),
            recorded: AtomicU64::new(0),
        }
    }
}

impl Share {
    fn is_sized(&self) -> bool {
        self.sized.load(Ordering::Relaxed)
    }
}

/// 一次任务运行里所有文件共享的进度汇总。
///
/// 分两类分别记账：
///
/// * **已知大小**的项：字节进度 `KD / KB`（已下载 / 总量）；
/// * **不知道大小**的项：数量进度 `UD / U`（已完成个数 / 未知项总数），字节按
///   「下载多少算多少」——它的已下载字节同时进分子和分母。
///
/// 整体百分比按**文件数占比**加权两类各自的进度：
///
/// `percent = UD / N + (K / N) · (KD / KB)`
///
/// （`N` 总数，`U` 当前未知数，`K = N - U`）。当 `N == K`（都已知道大小）时退化成
/// 纯字节百分比；当 `N == U` 时退化成纯数量百分比。字节显示为 `(KD+UB) / (KB+UB)`。
///
/// 总量**不预填**：从 0 开始，随每个文件从服务器实际拿到的 `Content-Length` /
/// `Content-Range` 动态转入「已知」组（`record_size`），整项失败时回滚
/// （`rollback_item`）。
struct Tracker {
    files_total: usize,
    /// 所有处理完的文件（成功或失败），用于 `X/N 个文件`。
    files_done: AtomicUsize,
    /// 还不知道实际大小、也没处理完的项数 `U`（含还没开始下载的）。
    unsized_files: AtomicUsize,
    /// 未知大小且已处理完的项数 `UD`。
    unsized_done_files: AtomicUsize,
    /// 已知大小项的已下载字节 `KD`。
    sized_done_bytes: AtomicU64,
    /// 已知大小项的总字节 `KB`。
    sized_total_bytes: AtomicU64,
    /// 未知大小项的已下载字节（同时进分子分母）。
    unsized_done_bytes: AtomicU64,
    last: Mutex<Instant>,
}

impl Tracker {
    fn new(files_total: usize) -> Self {
        Self {
            files_total,
            files_done: AtomicUsize::new(0),
            unsized_files: AtomicUsize::new(files_total),
            unsized_done_files: AtomicUsize::new(0),
            sized_done_bytes: AtomicU64::new(0),
            sized_total_bytes: AtomicU64::new(0),
            unsized_done_bytes: AtomicU64::new(0),
            last: Mutex::new(Instant::now()),
        }
    }

    /// 学到某一项的实际大小：把它记进「已知」组。
    ///
    /// 重复调用幂等，实际值变了就修正（重试 / 服务器前后不一致都安全）。转组发生在
    /// 读取正文之前（`single` / `chunked` 都是先拿到大小再下字节），所以转组时这一项
    /// 还没有字节记在「未知」组里。
    fn record_size(&self, share: &Share, actual: u64) {
        let prev = share.recorded.swap(actual, Ordering::Relaxed);
        match actual.cmp(&prev) {
            std::cmp::Ordering::Greater => {
                self.sized_total_bytes
                    .fetch_add(actual - prev, Ordering::Relaxed);
            }
            std::cmp::Ordering::Less => {
                self.sized_total_bytes
                    .fetch_sub(prev - actual, Ordering::Relaxed);
            }
            std::cmp::Ordering::Equal => {}
        }
        if !share.sized.swap(true, Ordering::Relaxed) {
            self.unsized_files.fetch_sub(1, Ordering::Relaxed);
        }
    }

    /// 某一项彻底失败：把它已记入「已知」组的字节从总量里回滚。
    fn rollback_item(&self, share: &Share) {
        let amount = share.recorded.swap(0, Ordering::Relaxed);
        if amount > 0 {
            self.sized_total_bytes.fetch_sub(amount, Ordering::Relaxed);
        }
    }

    /// 累计下载字节：按这一项在「已知 / 未知」哪组分别记账。
    fn add(&self, bytes: u64, sized: &AtomicBool, ctx: &TaskContext) {
        if sized.load(Ordering::Relaxed) {
            self.sized_done_bytes.fetch_add(bytes, Ordering::Relaxed);
        } else {
            self.unsized_done_bytes.fetch_add(bytes, Ordering::Relaxed);
        }
        self.maybe_report(ctx);
    }

    /// 重试失败时把这一轮已经上报的字节减回去。
    fn rollback(&self, bytes: u64, sized: &AtomicBool) {
        if sized.load(Ordering::Relaxed) {
            self.sized_done_bytes.fetch_sub(bytes, Ordering::Relaxed);
        } else {
            self.unsized_done_bytes.fetch_sub(bytes, Ordering::Relaxed);
        }
    }

    /// 一个文件处理完（成功或失败）。
    fn file_finished(&self, sized: bool, ctx: &TaskContext) {
        self.files_done.fetch_add(1, Ordering::Relaxed);
        if !sized {
            self.unsized_done_files.fetch_add(1, Ordering::Relaxed);
        }
        self.maybe_report(ctx);
    }

    fn message(&self) -> String {
        format!(
            "{}/{} 个文件",
            self.files_done.load(Ordering::Relaxed),
            self.files_total
        )
    }

    /// 整体百分比（0.0~100.0），见类型文档的公式。
    fn percent(&self) -> f64 {
        if self.files_total == 0 {
            return 0.0;
        }
        let total = self.files_total as f64;
        let unsized_count = self.unsized_files.load(Ordering::Relaxed) as f64;
        let sized = total - unsized_count;
        let unsized_done = self.unsized_done_files.load(Ordering::Relaxed) as f64;
        let sized_total = self.sized_total_bytes.load(Ordering::Relaxed) as f64;
        let sized_done = self.sized_done_bytes.load(Ordering::Relaxed) as f64;
        let sized_progress = if sized_total > 0.0 {
            (sized_done / sized_total).clamp(0.0, 1.0)
        } else {
            0.0
        };
        (((unsized_done + sized * sized_progress) / total) * 100.0).clamp(0.0, 100.0)
    }

    /// 当前进度快照（百分比用 [`Tracker::percent`]，字节按两类合并显示）。
    fn snapshot(&self) -> Progress {
        let unsized_bytes = self.unsized_done_bytes.load(Ordering::Relaxed);
        let current = self.sized_done_bytes.load(Ordering::Relaxed) + unsized_bytes;
        let total = self.sized_total_bytes.load(Ordering::Relaxed) + unsized_bytes;
        Progress::new(current, Some(total), self.message(), Some(self.percent()))
    }

    /// 开跑时立刻上报一次，先把文件数量显示出来（此时总量可能还是 0）。
    fn initial_report(&self, ctx: &TaskContext) {
        ctx.report(self.snapshot());
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
            ctx.report(self.snapshot());
        }
    }

    /// 收尾时强制上报一次，并给出汇总。
    fn finish(&self, ctx: &TaskContext, report: &DownloadReport) {
        if self.files_total > 0 {
            ctx.report(self.snapshot());
        }
        ctx.log(format!(
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
    /// 这一项是否已经知道实际大小（共享给分块任务的同一个开关）。
    sized: Arc<AtomicBool>,
    attempted: Arc<AtomicU64>,
}

impl Recorder {
    fn new(tracker: Arc<Tracker>, ctx: TaskContext, sized: Arc<AtomicBool>) -> Self {
        Self {
            tracker,
            ctx,
            sized,
            attempted: Arc::new(AtomicU64::new(0)),
        }
    }

    fn add(&self, bytes: u64) {
        self.attempted.fetch_add(bytes, Ordering::Relaxed);
        self.tracker.add(bytes, &self.sized, &self.ctx);
    }

    /// 成功：这一轮的字节保留在整体进度里。
    fn commit(&self) {
        self.attempted.store(0, Ordering::Relaxed);
    }

    /// 失败：把这一轮上报过的字节从整体进度里抹掉。
    fn discard(&self) {
        let bytes = self.attempted.swap(0, Ordering::Relaxed);
        self.tracker.rollback(bytes, &self.sized);
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

    /// 混合「已知 / 未知大小」的进度模型：百分比按文件数占比加权两类。
    #[test]
    fn the_tracker_blends_known_and_unknown_sizes() {
        let ctx = TaskContext::detached("下载");
        // N=2：一个已知大小、一个未知
        let tracker = Tracker::new(2);
        let known = Share::default();
        let unknown = Share::default();

        // 未知项先下了 10 字节；已知项学到 100 字节、下了 40
        tracker.add(10, &unknown.sized, &ctx);
        tracker.record_size(&known, 100);
        tracker.add(40, &known.sized, &ctx);

        // percent = UD/N + (K/N)(KD/KB) = 0/2 + (1/2)(40/100) = 20%
        assert!((tracker.percent() - 20.0).abs() < 1e-9);
        let snapshot = tracker.snapshot();
        // 字节 = (KD+UB) / (KB+UB) = (40+10) / (100+10)
        assert_eq!(snapshot.current, 50);
        assert_eq!(snapshot.total, Some(110));
        assert_eq!(snapshot.percent, Some(20.0));

        // 未知项完成 → UD=1 → percent = 1/2 + (1/2)(0.4) = 70%
        tracker.file_finished(false, &ctx);
        assert!((tracker.percent() - 70.0).abs() < 1e-9);
        assert_eq!(tracker.message(), "1/2 个文件");
    }

    /// 已知项的字节累加幂等、可修正，失败可整项回滚；学到大小时从「未知」转组。
    #[test]
    fn record_size_is_idempotent_correctable_and_rollbackable() {
        let tracker = Tracker::new(3);
        let share = Share::default();

        tracker.record_size(&share, 100);
        tracker.record_size(&share, 100); // 重试重记：幂等
        assert_eq!(tracker.sized_total_bytes.load(Ordering::Relaxed), 100);
        assert_eq!(tracker.unsized_files.load(Ordering::Relaxed), 2);

        tracker.record_size(&share, 130); // 服务器前后不一致：修正
        assert_eq!(tracker.sized_total_bytes.load(Ordering::Relaxed), 130);
        assert_eq!(tracker.unsized_files.load(Ordering::Relaxed), 2);

        tracker.rollback_item(&share); // 整项失败：回滚
        assert_eq!(tracker.sized_total_bytes.load(Ordering::Relaxed), 0);
    }

    /// 纯未知大小的项：百分比只按数量走，字节「下载多少算多少」。
    #[test]
    fn unknown_size_progress_is_count_based_with_self_growing_bytes() {
        let ctx = TaskContext::detached("下载");
        let tracker = Tracker::new(1);
        let share = Share::default();

        assert_eq!(tracker.percent(), 0.0);
        let snapshot = tracker.snapshot();
        assert_eq!((snapshot.current, snapshot.total), (0, Some(0)));

        // 下了 1 MiB 但还没完成：百分比仍是 0%，字节变成 1 MiB / 1 MiB
        tracker.add(1024 * 1024, &share.sized, &ctx);
        assert_eq!(tracker.percent(), 0.0);
        let snapshot = tracker.snapshot();
        assert_eq!(snapshot.current, 1024 * 1024);
        assert_eq!(snapshot.total, Some(1024 * 1024));

        // 完成 → 100%
        tracker.file_finished(false, &ctx);
        assert_eq!(tracker.percent(), 100.0);
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
    fn candidates_rotate_across_attempts() {
        let two = vec!["a".to_string(), "b".to_string()];
        assert_eq!(candidate_index(&two, 1), 0); // A
        assert_eq!(candidate_index(&two, 2), 1); // B
        assert_eq!(candidate_index(&two, 3), 0); // 回到 A

        let one = vec!["only".to_string()];
        assert_eq!(candidate_index(&one, 1), 0);
        assert_eq!(candidate_index(&one, 3), 0);
    }

    /// 没有候选地址的项直接报错，不会去碰网络。
    #[tokio::test]
    async fn an_item_without_candidates_fails_fast() {
        let task = DownloadTask::new(DownloadOptions::default()).with_item(DownloadItem::new(
            Vec::<String>::new(),
            "/tmp/dmcl-no-candidate",
        ));
        let err = task
            .execute(&TaskContext::detached("下载"))
            .await
            .unwrap_err();
        assert!(format!("{err:#}").contains("没有候选地址"), "{err:#}");
    }

    #[test]
    fn the_task_reports_its_items() {
        let mut task = DownloadTask::new(DownloadOptions::default())
            .named("下载游戏")
            .with_item(
                DownloadItem::new(["https://example.com/a.jar".to_string()], "/tmp/a.jar")
                    .with_size(Some(10)),
            );
        task.push(DownloadItem::new(
            ["https://example.com/b.jar".to_string()],
            "/tmp/b.jar",
        ));
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

    /// 重试时进度要回滚：一次失败尝试把已上报的字节减回去。
    #[test]
    fn a_recorder_rolls_back_a_discarded_attempt() {
        let ctx = TaskContext::detached("下载");
        let tracker = Arc::new(Tracker::new(1));
        let share = Share::default();
        let recorder = Recorder::new(tracker.clone(), ctx, share.sized.clone());

        recorder.add(40);
        recorder.add(60);
        assert_eq!(tracker.unsized_done_bytes.load(Ordering::Relaxed), 100);

        // 这次尝试作废，字节要抹掉
        recorder.discard();
        assert_eq!(tracker.unsized_done_bytes.load(Ordering::Relaxed), 0);

        // 成功的尝试则保留
        let recorder = Recorder::new(
            tracker.clone(),
            TaskContext::detached("下载"),
            share.sized.clone(),
        );
        recorder.add(50);
        recorder.commit();
        assert_eq!(tracker.unsized_done_bytes.load(Ordering::Relaxed), 50);
    }
}
