//! natives 的下载与解压。

use std::sync::Arc;

use anyhow::Result;
use dmcl_download::DownloadContext;
use dmcl_mcmeta::layout::natives_dir;
use dmcl_mcmeta::os::{Arch, Os};
use dmcl_task::TaskContext;

use crate::extract::extract_zip;
use crate::options::{download_planned, GameFilesOptions};
use crate::plan::{plan_native_jars, plan_natives};
use crate::version::ResolvedVersion;

/// 下载该 os / arch 需要的 natives 分类器 jar，并解压到 [`natives_dir`]。
///
/// 缺失的 jar 跳过并记一条日志：它通常会在完整性检查里被发现，这里不把「库还没下」
/// 当成 natives 解压失败。
pub async fn ensure_natives(
    version: &ResolvedVersion,
    os: Os,
    arch: Arch,
    options: &GameFilesOptions,
    downloads: &Arc<DownloadContext>,
    ctx: &TaskContext,
) -> Result<()> {
    let planned = plan_native_jars(&version.reference, &version.json, os, arch);
    download_planned(options, downloads, "natives", planned, ctx).await?;

    let target = natives_dir(version.reference.name.as_str(), os, arch);
    for (jar, name) in plan_natives(&version.reference, &version.json, os, arch) {
        if !jar.is_file() {
            ctx.log(format!("跳过缺失的 natives: {name}"));
            continue;
        }
        ctx.log(format!("解压 {name}"));
        extract_zip(jar, target.clone()).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::io::{Cursor, Write};
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
            std::env::temp_dir().join(format!("dmcl-game-natives-{tag}-{}", std::process::id()));
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

    async fn write_version(root: &Path, name: &str, json: serde_json::Value) {
        let path = version_json_path(root, name);
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&path, serde_json::to_string(&json).unwrap())
            .await
            .unwrap();
    }

    fn zip_with(file: &str, content: &[u8]) -> Vec<u8> {
        let mut buffer = Vec::new();
        {
            let mut writer = zip::ZipWriter::new(Cursor::new(&mut buffer));
            let options: zip::write::FileOptions<'_, '_, ()> = zip::write::FileOptions::default();
            writer.start_file(file, options).unwrap();
            writer.write_all(content).unwrap();
            writer.finish().unwrap();
        }
        buffer
    }

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
    async fn the_platform_native_is_downloaded_and_extracted() {
        let jar = zip_with("libfoo.so", b"native bytes");
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut responses = HashMap::new();
        responses.insert("/natives.jar".to_string(), jar.clone());
        let server = tokio::spawn(serve(listener, Arc::new(responses)));

        let root = work_dir("extract");
        let name = format!("natives-{}", std::process::id());
        let os = Os::current();
        let arch = Arch::X86_64;
        write_version(
            &root,
            &name,
            serde_json::json!({
                "id": name,
                "libraries": [{
                    "name": "x:x:1",
                    "natives": { os.mc_name(): "natives-test" },
                    "downloads": {"classifiers": {
                        "natives-test": {
                            "url": format!("http://{addr}/natives.jar"),
                            "path": "x/x/1/x-1-natives.jar",
                            "sha1": hash_bytes(HashAlgorithm::Sha1, &jar),
                            "size": jar.len()
                        }
                    }}
                }]
            }),
        )
        .await;

        let version = Arc::new(
            resolve_version(&VersionRef::new(&root, &name))
                .await
                .unwrap(),
        );
        let options = options();
        let downloads = downloads(&options);
        run_node(move |ctx| {
            let version = version.clone();
            let options = options.clone();
            let downloads = downloads.clone();
            async move { ensure_natives(&version, os, arch, &options, &downloads, &ctx).await }
        })
        .await
        .unwrap();

        let target = natives_dir(&name, os, arch);
        assert_eq!(
            tokio::fs::read(target.join("libfoo.so")).await.unwrap(),
            b"native bytes"
        );

        server.abort();
        let _ = tokio::fs::remove_dir_all(&root).await;
        let _ = tokio::fs::remove_dir_all(target).await;
    }

    #[tokio::test]
    async fn a_missing_native_jar_is_skipped() {
        let root = work_dir("missing");
        let name = format!("natives-missing-{}", std::process::id());
        let os = Os::current();
        write_version(
            &root,
            &name,
            serde_json::json!({
                "id": name,
                "libraries": [{
                    "name": "x:x:1",
                    "natives": { os.mc_name(): "natives-test" },
                    "downloads": {"classifiers": {
                        "natives-test": {"path": "x/x/1/x-1-natives.jar"}
                    }}
                }]
            }),
        )
        .await;

        let version = Arc::new(
            resolve_version(&VersionRef::new(&root, &name))
                .await
                .unwrap(),
        );
        let options = options();
        let downloads = downloads(&options);
        run_node(move |ctx| {
            let version = version.clone();
            let options = options.clone();
            let downloads = downloads.clone();
            async move {
                ensure_natives(&version, os, Arch::X86_64, &options, &downloads, &ctx).await
            }
        })
        .await
        .unwrap();

        let _ = tokio::fs::remove_dir_all(&root).await;
        let _ = tokio::fs::remove_dir_all(natives_dir(&name, os, Arch::X86_64)).await;
    }
}
