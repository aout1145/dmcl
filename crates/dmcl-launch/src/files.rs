//! 游戏文件完整性配置与文件 / natives 准备任务。

use std::sync::Arc;

use anyhow::Result;
use dmcl_download::DownloadContext;
use dmcl_game::{
    check_integrity, ensure_files, ensure_natives, extract_natives, GameFilesOptions,
    ResolvedVersion,
};
use dmcl_java::JavaOption;
use dmcl_mcmeta::os::Os;
use dmcl_task::{Task, TaskContext};

/// 启动时对游戏文件完整性的处理方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GameIntegrity {
    /// 不检查，也不补全缺失的文件。
    Ignore,
    /// 检查文件；不完整时等用户确认后继续，不补全。
    Check,
    /// 检查并补全缺失或损坏的文件。
    #[default]
    Complete,
}

/// 按配置处理文件完整性：忽略、只检查，或检查并补全。
pub struct PrepareLaunchFilesTask {
    game: Arc<GameFilesOptions>,
    downloads: Arc<DownloadContext>,
    integrity: GameIntegrity,
}

impl PrepareLaunchFilesTask {
    pub fn new(
        game: Arc<GameFilesOptions>,
        downloads: Arc<DownloadContext>,
        integrity: GameIntegrity,
    ) -> Self {
        Self {
            game,
            downloads,
            integrity,
        }
    }
}

#[async_trait::async_trait]
impl Task<Arc<ResolvedVersion>, ResolvedVersion> for PrepareLaunchFilesTask {
    async fn run(
        &self,
        resolved: Arc<ResolvedVersion>,
        ctx: TaskContext,
    ) -> Result<ResolvedVersion> {
        if self.integrity == GameIntegrity::Ignore {
            ctx.log("跳过游戏完整性检查");
            return Ok((*resolved).clone());
        }

        let integrity = check_integrity(&resolved, &self.game).await?;
        ctx.log(integrity.summary());
        if integrity.is_complete() {
            return Ok((*resolved).clone());
        }

        if self.integrity == GameIntegrity::Complete {
            ensure_files(&resolved, &self.game, &self.downloads, &ctx).await?;
        } else {
            ctx.log(integrity.detail(10));
            ctx.confirm(format!("{}；继续启动可能失败", integrity.summary()))
                .await?;
        }
        Ok((*resolved).clone())
    }
}

/// 按当前平台的 os 与已选 java 的 arch 解压 natives；只有「检查并补全」才下载缺失的 jar。
///
/// natives 的分类器（`natives-linux-64` 之类）取决于运行游戏的 JVM 架构，所以必须用
/// 上一步选好的 java；这里不再选择 Java。安装流程已按当前平台下载好 natives jar，
/// 其余模式只解压本地已有的文件，不联网补全。
pub struct PrepareNativesTask {
    game: Arc<GameFilesOptions>,
    downloads: Arc<DownloadContext>,
    integrity: GameIntegrity,
}

impl PrepareNativesTask {
    pub fn new(
        game: Arc<GameFilesOptions>,
        downloads: Arc<DownloadContext>,
        integrity: GameIntegrity,
    ) -> Self {
        Self {
            game,
            downloads,
            integrity,
        }
    }
}

#[async_trait::async_trait]
impl Task<(Arc<ResolvedVersion>, Arc<JavaOption>), JavaOption> for PrepareNativesTask {
    async fn run(
        &self,
        (version, java): (Arc<ResolvedVersion>, Arc<JavaOption>),
        ctx: TaskContext,
    ) -> Result<JavaOption> {
        let os = Os::current();
        if self.integrity == GameIntegrity::Complete {
            ensure_natives(&version, os, java.arch, &self.game, &self.downloads, &ctx).await?;
        } else {
            extract_natives(&version, os, java.arch, &ctx).await?;
        }
        Ok((*java).clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dmcl_game::{resolve_version, VersionRef};
    use dmcl_mcmeta::source::DownloadSource;
    use dmcl_task::{task_fn, TaskEvent, TaskGraph};
    use std::path::Path;

    fn game_options() -> Arc<GameFilesOptions> {
        Arc::new(GameFilesOptions::new(DownloadSource::official()))
    }

    /// 一个缺少一个库、且该库没有下载地址的版本：完整性检查必然不完整，
    /// 补全也无从下载。
    async fn write_incomplete_version(root: &Path) {
        let json = serde_json::json!({
            "id": "1.0",
            "mainClass": "a.Main",
            "libraries": [{
                "name": "a:a:1",
                "downloads": {"artifact": {"path": "a/a/1/a-1.jar", "url": ""}}
            }]
        });
        let path = dmcl_mcmeta::layout::version_json_path(root, "1.0");
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&path, serde_json::to_string(&json).unwrap())
            .await
            .unwrap();
    }

    fn files_graph(
        resolved: ResolvedVersion,
        game: Arc<GameFilesOptions>,
        integrity: GameIntegrity,
    ) -> TaskGraph<ResolvedVersion> {
        let mut builder = dmcl_task::TaskGraphBuilder::new("检查文件");
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
            "检查文件",
            source,
            PrepareLaunchFilesTask::new(game, downloads, integrity),
        );
        builder.build(sink).unwrap()
    }

    #[tokio::test]
    async fn ignoring_integrity_skips_the_check() {
        let root =
            std::env::temp_dir().join(format!("dmcl-integrity-ignore-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&root).await;
        write_incomplete_version(&root).await;
        let resolved = resolve_version(&VersionRef::new(&root, "1.0"))
            .await
            .unwrap();

        let graph = files_graph(resolved, game_options(), GameIntegrity::Ignore);
        let mut events = graph.subscribe();
        let running = tokio::spawn(async move { graph.run(|_| {}).await });
        let mut skipped = false;
        while let Some(event) = events.recv().await {
            if let TaskEvent::Log { message, .. } = event {
                skipped |= message.contains("跳过游戏完整性检查");
            }
        }
        running.await.unwrap().unwrap();
        assert!(skipped);

        tokio::fs::remove_dir_all(&root).await.unwrap();
    }

    #[tokio::test]
    async fn checking_integrity_waits_for_confirmation_when_incomplete() {
        let root =
            std::env::temp_dir().join(format!("dmcl-integrity-check-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&root).await;
        write_incomplete_version(&root).await;
        let resolved = resolve_version(&VersionRef::new(&root, "1.0"))
            .await
            .unwrap();

        let graph = files_graph(resolved, game_options(), GameIntegrity::Check);
        let mut events = graph.subscribe();
        let running = tokio::spawn(async move { graph.run(|_| {}).await });
        loop {
            match events.recv().await.unwrap() {
                TaskEvent::Warning {
                    message, decision, ..
                } => {
                    assert!(message.contains("版本不完整"), "{message}");
                    assert!(message.contains("继续启动可能失败"), "{message}");
                    decision.abort();
                    break;
                }
                _ => continue,
            }
        }
        let err = running.await.unwrap().unwrap_err();
        assert!(
            err.chain()
                .any(|cause| cause.downcast_ref::<dmcl_task::Cancelled>().is_some()),
            "{err:#}"
        );

        tokio::fs::remove_dir_all(&root).await.unwrap();
    }
}
