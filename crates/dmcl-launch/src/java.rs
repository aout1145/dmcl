//! Java 兼容性配置与选择任务。

use std::fmt;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use dmcl_download::DownloadContext;
use dmcl_game::{GameFilesOptions, ResolvedVersion};
use dmcl_java::{
    install_jre, JavaDetails, JavaInstalledHook, JavaOption, JavaResolver, JavaRuntimeOptions,
    JreAutoInstall, JreInstallRequest,
};
use dmcl_mcmeta::os::{Arch, Os};
use dmcl_task::{Task, TaskContext};

/// 启动时对所选 Java 的兼容性检查。
#[derive(Clone)]
pub enum JvmCompatibility {
    /// 不检查 Java 版本与架构。
    Ignore,
    /// 检查 Java 版本与架构；不匹配时等用户确认。
    Check,
    /// 自动选择 / 下载 Java；只有这个分支需要安装 hook。
    Complete {
        /// JRE 安装成功后调用；`None` 表示不写回配置。
        hook: Option<Arc<dyn JavaInstalledHook>>,
    },
}

impl Default for JvmCompatibility {
    fn default() -> Self {
        Self::Complete { hook: None }
    }
}

impl JvmCompatibility {
    /// 是否允许在缺 Java 或版本不匹配时自动下载 JRE。
    pub fn allows_auto_install(&self) -> bool {
        matches!(self, Self::Complete { .. })
    }

    /// 自动安装 JRE 成功后要调用的 hook。
    pub fn installed_hook(&self) -> Option<&Arc<dyn JavaInstalledHook>> {
        match self {
            Self::Complete { hook } => hook.as_ref(),
            Self::Ignore | Self::Check => None,
        }
    }
}

impl fmt::Debug for JvmCompatibility {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ignore => f.write_str("Ignore"),
            Self::Check => f.write_str("Check"),
            Self::Complete { hook } => f
                .debug_struct("Complete")
                .field("hook", &hook.is_some())
                .finish(),
        }
    }
}

/// 通过解析器选好 java；允许自动安装时下载匹配的 JRE；版本 / 架构仍不匹配时先等用户确认。
pub struct SelectJavaTask {
    java: Arc<dyn JavaResolver>,
    compatibility: JvmCompatibility,
    game: Arc<GameFilesOptions>,
    downloads: Arc<DownloadContext>,
    jre: Option<JreAutoInstall>,
}

impl SelectJavaTask {
    pub fn new(
        java: Arc<dyn JavaResolver>,
        compatibility: JvmCompatibility,
        game: Arc<GameFilesOptions>,
        downloads: Arc<DownloadContext>,
        jre: Option<JreAutoInstall>,
    ) -> Self {
        Self {
            java,
            compatibility,
            game,
            downloads,
            jre,
        }
    }

    /// 下载匹配版本要求的 Mojang JRE，并把它交给 hook（GUI 会写回配置）。
    async fn auto_install_java(
        &self,
        preferred: Option<&str>,
        required_major: u32,
        jre: &JreAutoInstall,
        ctx: &TaskContext,
    ) -> Result<JavaOption> {
        let arch =
            Arch::current().ok_or_else(|| anyhow!("无法确定当前 CPU 架构，不能自动下载 Java"))?;
        let runtime = JavaRuntimeOptions {
            source: self.game.source.clone(),
            download: self.game.download.clone(),
        };
        let mut request = JreInstallRequest::new(
            jre.runtime_root.clone(),
            (Os::current(), arch),
            required_major,
        )
        .with_format(jre.format);
        if let Some(component) = preferred.filter(|component| !component.is_empty()) {
            request = request.with_component(component);
        }

        let installed = install_jre(&runtime, &request, &self.downloads, ctx).await?;
        let details = JavaDetails::probe(installed.java).await?;
        if let Some(hook) = self.compatibility.installed_hook() {
            hook.on_installed(&details).await?;
        }
        ctx.log(format!(
            "已下载 Java {}：{}",
            details.option.major,
            details.option.path.display()
        ));
        Ok(details.option)
    }
}

/// 大版本与当前平台架构都匹配才算合适。
fn java_is_suitable(java: &JavaOption, required_major: u32) -> bool {
    java.major == required_major && Arch::current().is_none_or(|host| host == java.arch)
}

#[async_trait::async_trait]
impl Task<Arc<ResolvedVersion>, JavaOption> for SelectJavaTask {
    async fn run(&self, resolved: Arc<ResolvedVersion>, ctx: TaskContext) -> Result<JavaOption> {
        let required_major = resolved.json.required_java_major();
        let preferred = resolved
            .json
            .java_version
            .as_ref()
            .and_then(|java| java.component.clone())
            .filter(|component| !component.is_empty());

        let compatibility = &self.compatibility;
        let mut downloaded = false;
        let mut java = match self.java.resolve(required_major).await {
            Ok(java) => java,
            Err(err) => {
                if !compatibility.allows_auto_install() {
                    return Err(err);
                }
                let Some(jre) = &self.jre else {
                    return Err(err);
                };
                ctx.log(format!("没有找到合适的 Java：{err:#}，尝试自动下载"));
                downloaded = true;
                self.auto_install_java(preferred.as_deref(), required_major, jre, &ctx)
                    .await?
            }
        };

        if matches!(compatibility, JvmCompatibility::Ignore) {
            ctx.log(format!(
                "使用 Java {}：{}（忽略兼容性检查）",
                java.major,
                java.path.display()
            ));
            return Ok(java);
        }

        if !downloaded
            && compatibility.allows_auto_install()
            && !java_is_suitable(&java, required_major)
        {
            if let Some(jre) = &self.jre {
                match self
                    .auto_install_java(preferred.as_deref(), required_major, jre, &ctx)
                    .await
                {
                    Ok(installed) => java = installed,
                    Err(err) => ctx.log(format!(
                        "警告：自动下载 Java 失败，继续使用已选的 Java：{err:#}"
                    )),
                }
            }
        }

        ctx.log(format!(
            "使用 Java {}：{}（{} / {}）",
            java.major,
            java.path.display(),
            Os::current().mc_name(),
            java.arch.name()
        ));

        let mut mismatches = Vec::new();
        if java.major != required_major {
            mismatches.push(format!(
                "Java 版本不匹配（需要 {required_major}，实际为 {}）",
                java.major
            ));
        }
        if let Some(host) = Arch::current().filter(|host| *host != java.arch) {
            mismatches.push(format!(
                "Java 架构与当前平台不一致（Java 为 {}，平台为 {}）",
                java.arch.name(),
                host.name()
            ));
        }
        if !mismatches.is_empty() {
            ctx.confirm(mismatches.join("；")).await?;
        }
        Ok(java)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dmcl_game::VersionRef;
    use dmcl_mcmeta::source::DownloadSource;
    use dmcl_task::{task_fn, TaskEvent, TaskGraphBuilder};
    use std::path::PathBuf;

    struct NoopHook;

    #[async_trait::async_trait]
    impl JavaInstalledHook for NoopHook {
        async fn on_installed(&self, _details: &JavaDetails) -> Result<()> {
            Ok(())
        }
    }

    fn game_options() -> Arc<GameFilesOptions> {
        Arc::new(GameFilesOptions::new(DownloadSource::official()))
    }

    #[test]
    fn complete_allows_auto_install_and_carries_the_hook() {
        let default = JvmCompatibility::default();
        assert!(default.allows_auto_install());
        assert!(default.installed_hook().is_none());

        let hook: Arc<dyn JavaInstalledHook> = Arc::new(NoopHook);
        let complete = JvmCompatibility::Complete { hook: Some(hook) };
        assert!(complete.allows_auto_install());
        assert!(complete.installed_hook().is_some());
    }

    #[test]
    fn ignore_and_check_never_allow_auto_install() {
        for compatibility in [JvmCompatibility::Ignore, JvmCompatibility::Check] {
            assert!(!compatibility.allows_auto_install());
            assert!(compatibility.installed_hook().is_none());
        }
    }

    #[tokio::test]
    async fn selecting_a_mismatched_java_waits_for_confirmation() {
        let root = std::env::temp_dir().join(format!("dmcl-select-java-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&root).await;

        let json = serde_json::json!({
            "id": "1.0",
            "javaVersion": {"majorVersion": 17},
            "mainClass": "com.example.Main",
            "arguments": {"jvm": [], "game": []}
        });
        let path = dmcl_mcmeta::layout::version_json_path(&root, "1.0");
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&path, serde_json::to_string(&json).unwrap())
            .await
            .unwrap();

        let arch = dmcl_mcmeta::os::Arch::current().unwrap_or(dmcl_mcmeta::os::Arch::X86_64);
        let java = JavaOption {
            path: PathBuf::from("/opt/jdk8/bin/java"),
            major: 8,
            arch,
        };
        let resolved = dmcl_game::resolve_version(&VersionRef::new(&root, "1.0"))
            .await
            .unwrap();

        let game = game_options();
        let mut builder = TaskGraphBuilder::new("选择 Java");
        let step = builder.step("步骤");
        let source = builder.add(
            step,
            "版本",
            (),
            task_fn(move |_: (), _ctx| {
                let resolved = resolved.clone();
                async move { Ok(resolved) }
            }),
        );
        let downloads = game.download_context(builder.progress());
        let sink = builder.add(
            step,
            "选择 Java",
            source,
            SelectJavaTask::new(
                Arc::new(java),
                JvmCompatibility::Check,
                game,
                downloads,
                None,
            ),
        );
        let graph = builder.build(sink).unwrap();
        let mut events = graph.subscribe();
        let waiting = tokio::spawn(async move { graph.run(|_| {}).await });

        loop {
            match events.recv().await.unwrap() {
                TaskEvent::Warning {
                    message, decision, ..
                } => {
                    assert!(message.contains("需要 17"), "{message}");
                    assert!(message.contains("实际为 8"), "{message}");
                    decision.abort();
                    break;
                }
                _ => continue,
            }
        }
        let err = waiting.await.unwrap().unwrap_err();
        assert!(
            err.chain()
                .any(|cause| cause.downcast_ref::<dmcl_task::Cancelled>().is_some()),
            "{err:#}"
        );

        tokio::fs::remove_dir_all(&root).await.unwrap();
    }
}
