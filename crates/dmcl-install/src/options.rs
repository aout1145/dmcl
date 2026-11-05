//! 安装配置。

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Result};
use dmcl_download::{DownloadItem, DownloadOptions, DownloadTask};
use dmcl_mcmeta::os::{Arch, Os};
use dmcl_mcmeta::source::DownloadSource;
use dmcl_task::TaskContext;

use crate::jre::{install_jre_for_major, JreAutoInstall, JreForMajorRequest};
use crate::zip::find_on_path;

/// 运行 Forge / NeoForge 安装器 processor 时需要 java，由调用者提供。
///
/// 实现只负责找**现有** Java；找不到时由 [`InstallOptions::java_for_or_install`] 按需下载。
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
    /// 找不到匹配 Java 时自动下载 JRE 的设置；`None` 表示不下载。
    pub jre: Option<JreAutoInstall>,
}

impl fmt::Debug for InstallOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InstallOptions")
            .field("root_path", &self.root_path)
            .field("source", &self.source)
            .field("download", &self.download)
            .field("skip_music", &self.skip_music)
            .field("jre", &self.jre)
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
            jre: None,
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

    /// 允许在找不到匹配 Java 时自动下载 JRE。
    pub fn with_jre_auto_install(mut self, jre: JreAutoInstall) -> Self {
        self.jre = Some(jre);
        self
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

    /// 先找现有 Java；找不到且配置了自动安装时，用当前任务上下文下载 JRE。
    ///
    /// 下载走 [`install_jre_for_major`]，因此进度会进入传进来的任务事件流；
    /// `preferred_component` 通常是版本 JSON 的 `javaVersion.component`。
    pub async fn java_for_or_install(
        &self,
        required_major: u32,
        preferred_component: Option<&str>,
        ctx: &TaskContext,
    ) -> Result<PathBuf> {
        match self.java_for(required_major).await {
            Ok(java) => Ok(java),
            Err(err) => {
                let Some(jre) = &self.jre else {
                    return Err(err);
                };
                let arch = Arch::current()
                    .ok_or_else(|| anyhow!("无法确定当前 CPU 架构，不能自动下载 Java"))?;
                ctx.log(format!("自动下载 Java {required_major}：{err:#}"));
                let mut request = JreForMajorRequest::new(
                    jre.runtime_root.clone(),
                    (Os::current(), arch),
                    required_major,
                )
                .with_format(jre.format);
                if let Some(component) =
                    preferred_component.filter(|component| !component.is_empty())
                {
                    request = request.with_preferred_component(component);
                }
                let installed = install_jre_for_major(self, &request, ctx).await?;
                Ok(installed.java)
            }
        }
    }
}

/// `.minecraft` 是否已经存在（仅用于打印提示）。
pub fn root_exists(root_path: &Path) -> bool {
    root_path.is_dir()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_default_java_provider_is_usable() {
        let options = InstallOptions::new("/tmp/mc", DownloadSource::official());
        // 只要不 panic 就行：具体路径取决于环境
        let path = options.java_for(17).await.unwrap();
        assert!(!path.as_os_str().is_empty());
    }
}
