//! 下载上下文与单文件任务。
//!
//! 每个文件是一个独立的 DAG 节点；并发由 [`DownloadContext`] 里的三层信号量控制：
//! 活跃文件数、全局连接数（同时约束分块）、CPU 密集任务（校验 / 预检）。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use path_clean::PathClean;
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, SemaphorePermit};
use tokio::task::JoinSet;

use dmcl_task::{human_bytes, Cancelled, DynamicHandle, ProgressHandle, StepId, Task, TaskContext};

use crate::hash::{files_ok_with, hash_file, FileCheck};
use crate::item::DownloadItem;
use crate::options::DownloadOptions;

/// 两次读之间的空闲时限：超过它就认为连接僵死，报错并交给重试。
const READ_TIMEOUT: Duration = Duration::from_secs(30);

/// 一份下载计划共享的客户端、并发预算与全局进度。
pub struct DownloadContext {
    client: reqwest::Client,
    options: DownloadOptions,
    file_semaphore: Arc<Semaphore>,
    connection_semaphore: Arc<Semaphore>,
    worker_semaphore: Arc<Semaphore>,
    progress: ProgressHandle,
}

impl DownloadContext {
    pub fn new(
        client: reqwest::Client,
        options: DownloadOptions,
        progress: ProgressHandle,
    ) -> Arc<Self> {
        let options = options.normalized();
        Arc::new(Self {
            client,
            file_semaphore: Arc::new(Semaphore::new(options.max_active_files)),
            connection_semaphore: Arc::new(Semaphore::new(options.max_connections)),
            worker_semaphore: Arc::new(Semaphore::new(options.max_workers)),
            progress,
            options,
        })
    }

    pub fn options(&self) -> &DownloadOptions {
        &self.options
    }

    /// 单文件下载任务（下载 → 校验 → rename）。失败直接返回错误，由 DAG FailFast 处理。
    pub fn task(self: &Arc<Self>, item: DownloadItem) -> impl Task<(), ()> + 'static {
        FileDownloadTask {
            context: self.clone(),
            item,
        }
    }

    /// 下载准备节点：预检本地文件、增加总字节数、为缺失文件动态创建下载任务。
    ///
    /// 返回文件任务的句柄；动态任务自动成为当前节点的依赖，调用方不需要逐个 `wait`
    /// 才能 join。但如果准备节点接下来要读取下载好的文件（资源索引、natives、JRE），
    /// 就必须先 `wait`，否则文件还没落盘。
    pub async fn prepare(
        self: &Arc<Self>,
        ctx: &TaskContext,
        step: StepId,
        items: Vec<DownloadItem>,
    ) -> Result<Vec<DynamicHandle<()>>> {
        for item in &items {
            if item.urls.is_empty() {
                anyhow::bail!("下载项没有候选地址：{}", item.save_to.display());
            }
        }
        if items.is_empty() {
            return Ok(Vec::new());
        }

        let checks = items.iter().map(|item| FileCheck {
            path: absolute(&item.save_to),
            checksum: item.checksum.clone(),
            size: item.size,
        });
        let cancel = ctx.cancel_handle();
        let oks = tokio::select! {
            checked = files_ok_with(checks, self.worker_semaphore.clone()) => checked?,
            _ = cancel.cancelled() => return Err(anyhow::Error::new(Cancelled)),
        };

        let mut missing = Vec::new();
        let mut skipped = 0usize;
        let mut total = 0i64;
        for (item, ok) in items.into_iter().zip(oks) {
            if ok {
                skipped += 1;
                continue;
            }
            if let Some(size) = item.size {
                total = total.saturating_add(to_i64(size));
            }
            missing.push(item);
        }

        if skipped > 0 {
            ctx.log(format!("跳过 {skipped} 个已完整的文件"));
        }
        if missing.is_empty() {
            return Ok(Vec::new());
        }

        self.progress.add_total(total);
        ctx.log(format!("待下载 {} 个文件", missing.len()));
        let mut handles = Vec::with_capacity(missing.len());
        for item in missing {
            let name = format!("下载 {}", item.name);
            handles.push(ctx.add_task(step, name, (), self.task(item))?);
        }
        Ok(handles)
    }

    /// 用下载上下文的 worker 预算并发检查本地文件。
    pub async fn files_ok(&self, files: impl IntoIterator<Item = FileCheck>) -> Result<Vec<bool>> {
        files_ok_with(files, self.worker_semaphore.clone()).await
    }

    /// 一个文件从第一次尝试到成功 / 耗尽重试。
    async fn download_file(&self, item: &DownloadItem, ctx: &TaskContext) -> Result<u64> {
        let save_to = absolute(&item.save_to);
        let mut last_error: Option<anyhow::Error> = None;
        let total = TotalContribution::new(item.size, self.progress.clone());
        let attempts = self.options.max_retries.saturating_add(1);

        for attempt in 1..=attempts {
            ctx.check_cancelled()?;
            // 轮换候选源：第 n 次尝试用 urls[(n-1) % len]
            let url = item.urls[candidate_index(&item.urls, attempt)].clone();
            if attempt > 1 {
                ctx.log(format!(
                    "{} 重试（{}/{}）",
                    item.name,
                    attempt - 1,
                    self.options.max_retries
                ));
            }

            // 每次尝试单独记账：失败重试时把这一轮已经上报的字节减回去，
            // 否则进度会变成「100% 了还在涨」。
            let recorder = Recorder::new(self.progress.clone());
            match self
                .attempt(item, &url, &save_to, &recorder, &total, ctx)
                .await
            {
                Ok(bytes) => {
                    recorder.commit();
                    return Ok(bytes);
                }
                Err(error) => {
                    recorder.discard();
                    let _ = tokio::fs::remove_file(item.part_path()).await;
                    let cancelled = error
                        .chain()
                        .any(|cause| cause.downcast_ref::<Cancelled>().is_some());
                    if cancelled {
                        return Err(error);
                    }
                    last_error = Some(error.context(format!("下载 {} 失败", item.name)));
                }
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow!("下载 {} 失败", item.name)))
    }

    /// 一次尝试：下载 → 校验 → 改名。失败时不会留下最终文件。
    async fn attempt(
        &self,
        item: &DownloadItem,
        url: &str,
        save_to: &Path,
        recorder: &Recorder,
        total: &TotalContribution,
        ctx: &TaskContext,
    ) -> Result<u64> {
        if let Some(parent) = save_to.parent() {
            tokio::fs::create_dir_all(parent).await.ok();
        }
        let part = item.part_path();
        let _ = tokio::fs::remove_file(&part).await;

        // file 许可覆盖网络 + 写 `.part`，不覆盖最终哈希校验。
        let file_permit = acquire(&self.file_semaphore, ctx).await?;

        // 已知大小且足够大 → 先探测服务器是否支持 Range，再决定分块还是单连接
        let chunked_total = match item.size {
            Some(size) if self.options.should_chunk(size) => {
                let _connection = acquire(&self.connection_semaphore, ctx).await?;
                probe_range(&self.client, url, ctx).await?
            }
            _ => None,
        };

        let bytes = match chunked_total {
            Some(actual) => {
                total.observe(actual);
                self.chunked(url, &part, actual, recorder, ctx).await?
            }
            None => self.single(url, &part, recorder, total, ctx).await?,
        };

        // 网络阶段结束：先释放 file，再进入 CPU 校验。
        drop(file_permit);

        let worker_permit = acquire(&self.worker_semaphore, ctx).await?;
        if let Some(checksum) = item.checksum.as_ref() {
            let cancel = ctx.cancel_handle();
            let actual = tokio::select! {
                actual = hash_file(&part, checksum.algorithm()) => actual?,
                _ = cancel.cancelled() => return Err(anyhow::Error::new(Cancelled)),
            };
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
        drop(worker_permit);

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
        total: &TotalContribution,
        ctx: &TaskContext,
    ) -> Result<u64> {
        let _connection = acquire(&self.connection_semaphore, ctx).await?;
        let mut response = send_get(&self.client, url, None, ctx)
            .await?
            .error_for_status()?;
        if let Some(actual) = response.content_length() {
            total.observe(actual);
        }
        let mut file = tokio::fs::File::create(part).await?;
        let mut written = 0u64;
        while let Some(chunk) = next_chunk(&mut response, ctx).await? {
            file.write_all(&chunk).await?;
            written += chunk.len() as u64;
            recorder.add(chunk.len() as u64);
        }
        file.flush().await?;
        Ok(written)
    }

    /// 分块并发下载：每块占一个全局连接许可，单个大文件可以占满全部连接。
    async fn chunked(
        &self,
        url: &str,
        part: &Path,
        size: u64,
        recorder: &Recorder,
        ctx: &TaskContext,
    ) -> Result<u64> {
        // 预分配出目标大小：各分块各自 seek 到自己的偏移写入，互不干扰
        {
            let file = tokio::fs::File::create(part).await?;
            file.set_len(size).await?;
        }

        let chunk_size = self.options.chunk_size;
        let mut set: JoinSet<Result<u64>> = JoinSet::new();
        let mut written = 0u64;

        let mut start = 0u64;
        while start < size {
            // 先占一个全局连接名额，再 spawn 这一块；permit 随块任务释放
            let permit = acquire_owned(&self.connection_semaphore, ctx).await?;
            let end = (start + chunk_size - 1).min(size - 1);
            let request = ChunkRequest {
                client: self.client.clone(),
                url: url.to_string(),
                start,
                end,
                part: part.to_path_buf(),
                recorder: recorder.clone(),
                ctx: ctx.clone(),
            };
            set.spawn(async move { request.run(permit).await });
            start = end + 1;

            while let Some(joined) = set.try_join_next() {
                written += handle_chunk(joined, &mut set)?;
            }
        }

        while let Some(joined) = set.join_next().await {
            written += handle_chunk(joined, &mut set)?;
        }
        Ok(written)
    }
}

/// 单文件任务。
struct FileDownloadTask {
    context: Arc<DownloadContext>,
    item: DownloadItem,
}

#[async_trait::async_trait]
impl Task<(), ()> for FileDownloadTask {
    async fn run(&self, _input: (), ctx: TaskContext) -> Result<()> {
        let bytes = self.context.download_file(&self.item, &ctx).await?;
        ctx.log(format!("{} 完成（{}）", self.item.name, human_bytes(bytes)));
        Ok(())
    }
}

/// 处理一个已完成的分块；失败时中止其余分块。
fn handle_chunk(
    joined: std::result::Result<Result<u64>, tokio::task::JoinError>,
    set: &mut JoinSet<Result<u64>>,
) -> Result<u64> {
    match joined {
        Ok(Ok(bytes)) => Ok(bytes),
        Ok(Err(error)) => {
            set.abort_all();
            Err(error)
        }
        Err(join_error) => {
            set.abort_all();
            Err(anyhow!("分块下载任务异常终止: {join_error}"))
        }
    }
}

/// 一个分块请求；持有全局连接许可直到请求结束。
struct ChunkRequest {
    client: reqwest::Client,
    url: String,
    start: u64,
    end: u64,
    part: PathBuf,
    recorder: Recorder,
    ctx: TaskContext,
}

impl ChunkRequest {
    /// 抓取 `[start, end]` 区间的字节并写到文件的对应偏移。
    async fn run(self, _permit: OwnedSemaphorePermit) -> Result<u64> {
        let range = format!("bytes={}-{}", self.start, self.end);
        let mut response = send_get(&self.client, &self.url, Some(&range), &self.ctx)
            .await?
            .error_for_status()?;
        if response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
            anyhow::bail!("服务器没有按 Range 请求返回 206（{}）", response.status());
        }

        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .open(&self.part)
            .await?;
        file.seek(std::io::SeekFrom::Start(self.start)).await?;

        let mut written = 0u64;
        while let Some(chunk) = next_chunk(&mut response, &self.ctx).await? {
            file.write_all(&chunk).await?;
            written += chunk.len() as u64;
            self.recorder.add(chunk.len() as u64);
        }
        file.flush().await?;

        let expected = self.end - self.start + 1;
        if written != expected {
            anyhow::bail!(
                "分块 {}-{} 长度不符（期望 {expected}，实际 {written}）",
                self.start,
                self.end
            );
        }
        Ok(written)
    }
}

/// 发一个 GET 请求，等待响应头也有时限（防止服务器 accept 之后僵住不回包）。
async fn send_get(
    client: &reqwest::Client,
    url: &str,
    range: Option<&str>,
    ctx: &TaskContext,
) -> Result<reqwest::Response> {
    let mut request = client.get(url);
    if let Some(range) = range {
        request = request.header(reqwest::header::RANGE, range);
    }
    let cancel = ctx.cancel_handle();
    let sent = tokio::select! {
        sent = tokio::time::timeout(READ_TIMEOUT, request.send()) => {
            sent.map_err(|_| anyhow!("等待服务器响应超时（{} 秒）", READ_TIMEOUT.as_secs()))?
        }
        _ = cancel.cancelled() => return Err(anyhow::Error::new(Cancelled)),
    };
    Ok(sent?)
}

/// 读下一段响应体；取消时立刻返回。
async fn next_chunk(
    response: &mut reqwest::Response,
    ctx: &TaskContext,
) -> Result<Option<Vec<u8>>> {
    let cancel = ctx.cancel_handle();
    tokio::select! {
        chunk = response.chunk() => Ok(chunk?.map(|chunk| chunk.to_vec())),
        _ = cancel.cancelled() => Err(anyhow::Error::new(Cancelled)),
    }
}

/// 探测服务器是否支持 Range，返回资源总大小。
///
/// 只发一个 `bytes=0-0` 的请求；不支持（不是 206、或没有 `Content-Range`）时返回
/// `None`，调用者就退回单连接下载。
async fn probe_range(
    client: &reqwest::Client,
    url: &str,
    ctx: &TaskContext,
) -> Result<Option<u64>> {
    let response = send_get(client, url, Some("bytes=0-0"), ctx).await?;
    if response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
        return Ok(None);
    }
    let Some(header) = response
        .headers()
        .get(reqwest::header::CONTENT_RANGE)
        .and_then(|header| header.to_str().ok())
    else {
        return Ok(None);
    };
    Ok(parse_content_range(header))
}

/// 从 `bytes 0-0/1234` 里取出总长度。
fn parse_content_range(value: &str) -> Option<u64> {
    value.rsplit('/').next()?.trim().parse().ok()
}

/// 第 `attempt`（从 1 开始）次尝试该用哪个候选地址：按优先级轮换（A B A …）。
fn candidate_index(urls: &[String], attempt: u32) -> usize {
    (attempt.saturating_sub(1) as usize) % urls.len()
}

/// 可取消地获取一个信号量许可。
async fn acquire<'a>(semaphore: &'a Semaphore, ctx: &TaskContext) -> Result<SemaphorePermit<'a>> {
    let cancel = ctx.cancel_handle();
    tokio::select! {
        permit = semaphore.acquire() => Ok(permit.context("下载信号量已关闭")?),
        _ = cancel.cancelled() => Err(anyhow::Error::new(Cancelled)),
    }
}

/// 可取消地获取一个 owned 信号量许可（分块任务需要）。
async fn acquire_owned(
    semaphore: &Arc<Semaphore>,
    ctx: &TaskContext,
) -> Result<OwnedSemaphorePermit> {
    let cancel = ctx.cancel_handle();
    tokio::select! {
        permit = semaphore.clone().acquire_owned() => Ok(permit.context("下载信号量已关闭")?),
        _ = cancel.cancelled() => Err(anyhow::Error::new(Cancelled)),
    }
}

/// 一个文件在全局 `total` 里的当前贡献；服务器报告实际大小时修订。
struct TotalContribution {
    progress: ProgressHandle,
    current: AtomicI64,
}

impl TotalContribution {
    fn new(planned: Option<u64>, progress: ProgressHandle) -> Self {
        Self {
            progress,
            current: AtomicI64::new(planned.map(to_i64).unwrap_or(0)),
        }
    }

    /// 学到实际大小：把这一项的贡献从旧值修订成实际值（重复调用幂等）。
    fn observe(&self, actual: u64) {
        let actual = to_i64(actual);
        let previous = self.current.swap(actual, Ordering::SeqCst);
        self.progress.add_total(actual.saturating_sub(previous));
    }
}

/// 一次下载尝试的字节记账：累计进全局进度，失败重试时把那部分减回去。
#[derive(Clone)]
struct Recorder {
    progress: ProgressHandle,
    attempted: Arc<AtomicU64>,
}

impl Recorder {
    fn new(progress: ProgressHandle) -> Self {
        Self {
            progress,
            attempted: Arc::new(AtomicU64::new(0)),
        }
    }

    fn add(&self, bytes: u64) {
        self.attempted.fetch_add(bytes, Ordering::SeqCst);
        self.progress.add_downloaded(to_i64(bytes));
    }

    /// 成功：这一轮的字节保留在整体进度里。
    fn commit(&self) {
        self.attempted.store(0, Ordering::SeqCst);
    }

    /// 失败：把这一轮上报过的字节从整体进度里抹掉。
    fn discard(&self) {
        let bytes = self.attempted.swap(0, Ordering::SeqCst);
        if bytes > 0 {
            self.progress.add_downloaded(-to_i64(bytes));
        }
    }
}

fn to_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
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
    fn candidates_rotate_across_attempts() {
        let two = vec!["a".to_string(), "b".to_string()];
        assert_eq!(candidate_index(&two, 1), 0); // A
        assert_eq!(candidate_index(&two, 2), 1); // B
        assert_eq!(candidate_index(&two, 3), 0); // 回到 A

        let one = vec!["only".to_string()];
        assert_eq!(candidate_index(&one, 1), 0);
        assert_eq!(candidate_index(&one, 3), 0);
    }

    #[test]
    fn paths_are_normalised_like_everywhere_else() {
        let path = absolute(Path::new("a/./b/../c"));
        assert!(path.is_absolute());
        assert!(path.ends_with("a/c"), "{path:?}");
    }

    #[test]
    fn chunk_ranges_cover_the_file_exactly() {
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
    fn total_contribution_is_revised_idempotently() {
        let progress = ProgressHandle::new();
        // 预检已经按计划大小把 100 加进 total。
        progress.add_total(100);
        let total = TotalContribution::new(Some(100), progress.clone());
        assert_eq!(progress.snapshot().total, 100);

        total.observe(100);
        assert_eq!(progress.snapshot().total, 100);
        total.observe(100);
        assert_eq!(progress.snapshot().total, 100);
        total.observe(80);
        assert_eq!(progress.snapshot().total, 80);
        total.observe(130);
        assert_eq!(progress.snapshot().total, 130);
    }

    #[test]
    fn a_discarded_attempt_rolls_back_its_bytes() {
        let progress = ProgressHandle::new();
        let recorder = Recorder::new(progress.clone());
        recorder.add(40);
        recorder.add(60);
        assert_eq!(progress.snapshot().downloaded, 100);

        recorder.discard();
        assert_eq!(progress.snapshot().downloaded, 0);

        let recorder = Recorder::new(progress.clone());
        recorder.add(50);
        recorder.commit();
        assert_eq!(progress.snapshot().downloaded, 50);
    }

    #[test]
    fn a_retry_does_not_double_count_bytes() {
        let progress = ProgressHandle::new();
        let first = Recorder::new(progress.clone());
        first.add(100);
        first.discard();

        let second = Recorder::new(progress.clone());
        second.add(100);
        second.commit();
        assert_eq!(progress.snapshot().downloaded, 100);
    }

    #[tokio::test]
    async fn an_item_without_candidates_fails_before_creating_tasks() {
        let mut builder = dmcl_task::TaskGraphBuilder::new("测试");
        let step = builder.step("下载");
        let downloads = DownloadContext::new(
            reqwest::Client::new(),
            DownloadOptions::default(),
            ProgressHandle::new(),
        );
        let sink = builder.add(
            step,
            "准备",
            (),
            dmcl_task::task_fn(move |_: (), ctx| {
                let downloads = downloads.clone();
                async move {
                    downloads
                        .prepare(
                            &ctx,
                            step,
                            vec![DownloadItem::new(
                                Vec::<String>::new(),
                                std::env::temp_dir().join("dmcl-no-candidate"),
                            )],
                        )
                        .await
                        .map(|_| ())
                }
            }),
        );
        let error = builder.build(sink).unwrap().run(|_| {}).await.unwrap_err();
        assert!(format!("{error:#}").contains("没有候选地址"), "{error:#}");
    }
}
