//! 安装配置。

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use dmcl_download::DownloadOptions;
use dmcl_game::{GameFilesOptions, VersionRef};
use dmcl_java::{install_jre, JavaResolver, JavaRuntimeOptions, JreAutoInstall, JreInstallRequest};
use dmcl_mcmeta::os::{Arch, Os};
use dmcl_task::TaskContext;

/// 一次安装的全部外部配置。
#[derive(Clone)]
pub struct InstallOptions {
    /// `.minecraft` 根目录。
    pub root_path: PathBuf,
    /// 文件下载 / 规划相关的配置。
    pub game: GameFilesOptions,
    /// 跑安装器 processor 时怎么找 java。
    pub java: Arc<dyn JavaResolver>,
    /// 找不到匹配 Java 时自动下载 JRE 的设置。
    pub jre: JreAutoInstall,
}

impl fmt::Debug for InstallOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InstallOptions")
            .field("root_path", &self.root_path)
            .field("game", &self.game)
            .field("jre", &self.jre)
            .finish_non_exhaustive()
    }
}

impl InstallOptions {
    /// Java 与 JRE 安装位置都必须显式提供：安装器 processor 需要它们。
    pub fn new(
        root: impl Into<PathBuf>,
        game: GameFilesOptions,
        java: Arc<dyn JavaResolver>,
        jre: JreAutoInstall,
    ) -> Self {
        Self {
            root_path: root.into(),
            game,
            java,
            jre,
        }
    }

    pub fn with_download(mut self, download: DownloadOptions) -> Self {
        self.game.download = download;
        self
    }

    pub fn with_skip_music(mut self, skip_music: bool) -> Self {
        self.game.skip_music = skip_music;
        self
    }

    /// 当前 options 对应的版本引用。
    pub fn version(&self, name: impl Into<String>) -> VersionRef {
        VersionRef::new(self.root_path.clone(), name)
    }

    /// processor 用的 Java：先找现有，再按需下载。
    pub async fn java_for_or_install(
        &self,
        major: u32,
        component: Option<&str>,
        ctx: &TaskContext,
    ) -> Result<PathBuf> {
        match self.java.resolve(major).await {
            Ok(java) => Ok(java.path),
            Err(err) => {
                let arch = Arch::current()
                    .ok_or_else(|| anyhow!("无法确定当前 CPU 架构，不能自动下载 Java"))?;
                ctx.log(format!("自动下载 Java {major}：{err:#}"));

                let runtime = JavaRuntimeOptions {
                    source: self.game.source.clone(),
                    download: self.game.download.clone(),
                };
                let mut request = JreInstallRequest::new(
                    self.jre.runtime_root.clone(),
                    (Os::current(), arch),
                    major,
                )
                .with_format(self.jre.format);
                if let Some(component) = component.filter(|component| !component.is_empty()) {
                    request = request.with_component(component);
                }
                let installed = install_jre(&runtime, &request, ctx).await?;
                Ok(installed.java)
            }
        }
    }
}
