//! 确保一个版本需要的文件在本地就位（幂等，不删除额外文件）。

use std::sync::Arc;

use anyhow::{Context, Result};
use dmcl_download::DownloadContext;
use dmcl_mcmeta::vanilla::AssetIndexFile;
use dmcl_task::TaskContext;

use crate::options::{download_planned, GameFilesOptions};
use crate::plan::{plan_assets_index, plan_client_jar, plan_files};
use crate::version::ResolvedVersion;

/// 确保资源索引存在，并读出来。版本没有 `assetIndex` 时返回 `None`。
pub async fn ensure_asset_index(
    version: &ResolvedVersion,
    options: &GameFilesOptions,
    downloads: &Arc<DownloadContext>,
    ctx: &TaskContext,
) -> Result<Option<AssetIndexFile>> {
    let Some(planned) = plan_assets_index(&version.reference, &version.json) else {
        return Ok(None);
    };
    let path = planned.save_to.clone();
    download_planned(options, downloads, "资源索引", vec![planned], ctx).await?;

    let text = tokio::fs::read_to_string(&path)
        .await
        .with_context(|| format!("无法读取资源索引 {}", path.display()))?;
    let index = serde_json::from_str(&text)
        .with_context(|| format!("无法解析资源索引 {}", path.display()))?;
    Ok(Some(index))
}

/// 确保客户端 jar 存在（Forge / NeoForge processor 之前需要）。
pub async fn ensure_client_jar(
    version: &ResolvedVersion,
    options: &GameFilesOptions,
    downloads: &Arc<DownloadContext>,
    ctx: &TaskContext,
) -> Result<()> {
    let mut planned = Vec::new();
    if let Some(client) = plan_client_jar(&version.reference, &version.json) {
        planned.push(client);
    }
    download_planned(options, downloads, "客户端 jar", planned, ctx).await
}

/// 把缺的 / 坏的文件补齐；幂等，不删除额外文件。
pub async fn ensure_files(
    version: &ResolvedVersion,
    options: &GameFilesOptions,
    downloads: &Arc<DownloadContext>,
    ctx: &TaskContext,
) -> Result<()> {
    let index = ensure_asset_index(version, options, downloads, ctx).await?;
    let planned = plan_files(version, index.as_ref(), options);
    download_planned(options, downloads, "游戏文件", planned, ctx).await
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use dmcl_download::{hash_bytes, DownloadContext, DownloadOptions, HashAlgorithm};
    use dmcl_mcmeta::layout::version_json_path;
    use dmcl_mcmeta::source::{DownloadSource, Mirror};
    use dmcl_task::{task_fn, ProgressHandle, TaskContext, TaskGraphBuilder};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::*;
    use crate::version::{resolve_version, VersionRef};

    fn work_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("dmcl-game-ensure-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn options() -> GameFilesOptions {
        GameFilesOptions::new(DownloadSource::new(Mirror::Official))
            .with_download(DownloadOptions::default().with_max_retries(0))
    }

    fn downloads(options: &GameFilesOptions) -> Arc<DownloadContext> {
        options.download_context(ProgressHandle::new())
    }

    async fn run_node<F, Fut>(f: F) -> anyhow::Result<()>
    where
        F: Fn(TaskContext) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = anyhow::Result<()>> + Send + 'static,
    {
        let mut builder = TaskGraphBuilder::new("测试");
        let step = builder.step("测试");
        let sink = builder.add(step, "测试", (), task_fn(move |_: (), ctx| f(ctx)));
        builder.build(sink).unwrap().run(|_| {}).await.map(|_| ())
    }

    async fn write_version(root: &Path, client_url: &str, content: &[u8]) {
        let json = serde_json::json!({
            "id": "1.0",
            "downloads": {"client": {
                "url": client_url,
                "sha1": hash_bytes(HashAlgorithm::Sha1, content),
                "size": content.len()
            }}
        });
        let path = version_json_path(root, "1.0");
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&path, serde_json::to_string(&json).unwrap())
            .await
            .unwrap();
    }

    async fn resolved(root: &Path) -> ResolvedVersion {
        resolve_version(&VersionRef::new(root, "1.0"))
            .await
            .unwrap()
    }

    /// 一个只会回固定响应的极简 HTTP 服务器；路径不在表里就返回 404。
    async fn serve(listener: TcpListener, responses: Arc<HashMap<String, Vec<u8>>>) {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let responses = responses.clone();
            tokio::spawn(async move {
                let mut buffer = [0u8; 2048];
                let read = socket.read(&mut buffer).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..read]).into_owned();
                let path = request.split_whitespace().nth(1).unwrap_or("/");
                let body = responses.get(path);
                let (status, body): (&str, &[u8]) = match body {
                    Some(body) => ("200 OK", body),
                    None => ("404 Not Found", b""),
                };
                let header = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = socket.write_all(header.as_bytes()).await;
                let _ = socket.write_all(body).await;
            });
        }
    }

    #[tokio::test]
    async fn missing_files_are_downloaded_and_existing_files_are_skipped() {
        const CONTENT: &[u8] = b"client jar bytes";
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut responses = HashMap::new();
        responses.insert("/client.jar".to_string(), CONTENT.to_vec());
        let server = tokio::spawn(serve(listener, Arc::new(responses)));

        let root = work_dir("download");
        write_version(&root, &format!("http://{addr}/client.jar"), CONTENT).await;
        let version = Arc::new(resolved(&root).await);
        let options = options();
        let downloads = downloads(&options);

        let options_for_task = options.clone();
        let downloads_for_task = downloads.clone();
        let version_for_task = version.clone();
        run_node(move |ctx| {
            let options = options_for_task.clone();
            let downloads = downloads_for_task.clone();
            let version = version_for_task.clone();
            async move { ensure_files(&version, &options, &downloads, &ctx).await }
        })
        .await
        .unwrap();
        let jar = version.reference.jar_path();
        assert_eq!(tokio::fs::read(&jar).await.unwrap(), CONTENT);

        // 额外文件不属于版本规划，补全时不能被删掉。
        let extra = version.reference.dir().join("extra.txt");
        tokio::fs::write(&extra, b"keep").await.unwrap();

        let options_for_task = options.clone();
        let downloads_for_task = downloads.clone();
        let version_for_task = version.clone();
        run_node(move |ctx| {
            let options = options_for_task.clone();
            let downloads = downloads_for_task.clone();
            let version = version_for_task.clone();
            async move { ensure_files(&version, &options, &downloads, &ctx).await }
        })
        .await
        .unwrap();
        assert!(jar.is_file());
        assert!(extra.is_file());

        server.abort();
        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    #[tokio::test]
    async fn a_failed_download_is_an_error() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(serve(listener, Arc::new(HashMap::new())));

        let root = work_dir("failure");
        write_version(&root, &format!("http://{addr}/missing.jar"), b"x").await;
        let version = resolved(&root).await;
        let options = options();
        let downloads = downloads(&options);

        let err = run_node(move |ctx| {
            let options = options.clone();
            let downloads = downloads.clone();
            let version = version.clone();
            async move { ensure_files(&version, &options, &downloads, &ctx).await }
        })
        .await
        .unwrap_err();
        assert!(format!("{err:#}").contains("下载"), "{err:#}");

        server.abort();
        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    #[tokio::test]
    async fn ensure_client_jar_only_fetches_the_version_jar() {
        const CONTENT: &[u8] = b"client";
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut responses = HashMap::new();
        responses.insert("/client.jar".to_string(), CONTENT.to_vec());
        let server = tokio::spawn(serve(listener, Arc::new(responses)));

        let root = work_dir("client");
        write_version(&root, &format!("http://{addr}/client.jar"), CONTENT).await;
        let version = resolved(&root).await;
        let options = options();
        let downloads = downloads(&options);
        let version_for_task = version.clone();

        run_node(move |ctx| {
            let options = options.clone();
            let downloads = downloads.clone();
            let version = version_for_task.clone();
            async move { ensure_client_jar(&version, &options, &downloads, &ctx).await }
        })
        .await
        .unwrap();
        assert_eq!(
            tokio::fs::read(version.reference.jar_path()).await.unwrap(),
            CONTENT
        );

        server.abort();
        let _ = tokio::fs::remove_dir_all(&root).await;
    }
}
