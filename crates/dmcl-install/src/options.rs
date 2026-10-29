//! 安装配置。

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use dmcl_download::{DownloadItem, DownloadOptions, DownloadTask};
use dmcl_mcmeta::source::DownloadSource;

use crate::zip::find_on_path;

/// 运行 Forge / NeoForge 安装器 processor 时需要 java，由调用者提供。
pub trait JavaProvider: Send + Sync {
    /// 返回满足 `required_major` 的 java 可执行文件。
    fn java_for<'a>(&'a self, required_major: u32) -> dmcl_task::BoxFuture<'a, Result<PathBuf>>;
}

/// 明确的 java 路径。
#[derive(Debug, Clone)]
pub struct FixedJava(pub PathBuf);

impl JavaProvider for FixedJava {
    fn java_for<'a>(&'a self, _required_major: u32) -> dmcl_task::BoxFuture<'a, Result<PathBuf>> {
        Box::pin(async move { Ok(self.0.clone()) })
    }
}

/// 从 `JAVA_HOME` 或 `PATH` 上找 `java`（默认策略）。
#[derive(Debug, Clone, Default)]
pub struct PathJava;

impl JavaProvider for PathJava {
    fn java_for<'a>(&'a self, _required_major: u32) -> dmcl_task::BoxFuture<'a, Result<PathBuf>> {
        Box::pin(async move {
            if let Some(home) = std::env::var_os("JAVA_HOME") {
                let candidate = PathBuf::from(home).join("bin").join("java");
                if candidate.is_file() {
                    return Ok(candidate);
                }
            }
            // 交给操作系统在 PATH 里找
            Ok(find_on_path("java").unwrap_or_else(|| PathBuf::from("java")))
        })
    }
}

/// 一次安装的全部外部配置。
#[derive(Clone)]
pub struct InstallOptions {
    /// `.minecraft` 根目录。
    pub root_path: PathBuf,
    /// 下载源（镜像 + http 客户端）。
    pub source: DownloadSource,
    /// 文件下载的并发 / 分块策略。
    pub download: DownloadOptions,
    /// 是否跳过 `*.ogg` 资源（音乐与音效）。
    pub skip_music: bool,
    /// 跑安装器 processor 时用哪个 java。
    pub java: Arc<dyn JavaProvider>,
}

impl fmt::Debug for InstallOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InstallOptions")
            .field("root_path", &self.root_path)
            .field("source", &self.source)
            .field("download", &self.download)
            .field("skip_music", &self.skip_music)
            .finish_non_exhaustive()
    }
}

impl InstallOptions {
    pub fn new(root_path: impl Into<PathBuf>, source: DownloadSource) -> Self {
        Self {
            root_path: root_path.into(),
            source,
            download: DownloadOptions::default(),
            skip_music: false,
            java: Arc::new(PathJava),
        }
    }

    pub fn with_download(mut self, download: DownloadOptions) -> Self {
        self.download = download;
        self
    }

    pub fn with_skip_music(mut self, skip_music: bool) -> Self {
        self.skip_music = skip_music;
        self
    }

    pub fn with_java(mut self, java: Arc<dyn JavaProvider>) -> Self {
        self.java = java;
        self
    }

    /// 用固定路径的 java。
    pub fn with_java_path(self, path: impl Into<PathBuf>) -> Self {
        self.with_java(Arc::new(FixedJava(path.into())))
    }

    /// 只跟"本地有哪些文件"有关的那部分配置。
    pub fn plan(&self) -> crate::plan::PlanContext {
        crate::plan::PlanContext {
            root_path: self.root_path.clone(),
            skip_music: self.skip_music,
        }
    }

    /// 造一个下载任务：复用下载源的 http 客户端（同一连接池）。
    pub fn download_task(&self, items: Vec<DownloadItem>) -> DownloadTask {
        let mut task =
            DownloadTask::new(self.download.clone()).with_client(self.source.client().clone());
        task.extend(items);
        task
    }

    /// 跑安装器 processor 时求得 java 路径。
    pub async fn java_for(&self, required_major: u32) -> Result<PathBuf> {
        self.java.java_for(required_major).await
    }
}

/// `.minecraft` 是否已经存在（仅用于打印提示）。
pub fn root_exists(root_path: &Path) -> bool {
    root_path.is_dir()
}

#[cfg(test)]
mod tests {
    use super::*;
    use dmcl_mcmeta::source::Mirror;

    #[test]
    fn options_carry_the_callers_configuration() {
        let options = InstallOptions::new("/tmp/mc", DownloadSource::new(Mirror::Bmclapi))
            .with_skip_music(true)
            .with_java_path("/opt/java/bin/java");
        assert_eq!(options.root_path, PathBuf::from("/tmp/mc"));
        assert!(options.skip_music);
        assert_eq!(options.source.mirror(), Mirror::Bmclapi);
        assert!(format!("{options:?}").contains("/tmp/mc"));
    }

    #[tokio::test]
    async fn the_default_java_provider_is_usable() {
        let options = InstallOptions::new("/tmp/mc", DownloadSource::official());
        // 只要不 panic 就行：具体路径取决于环境
        let path = options.java_for(17).await.unwrap();
        assert!(!path.as_os_str().is_empty());

        let fixed =
            InstallOptions::new("/tmp/mc", DownloadSource::official()).with_java_path("/x/y/java");
        assert_eq!(fixed.java_for(8).await.unwrap(), PathBuf::from("/x/y/java"));
    }
}
