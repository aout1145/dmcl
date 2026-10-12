//! 启动流程：准备 → 补全 → 启动。
//!
//! * [`PrepareLaunchTask`]（`Task<(), PreparedLaunch>`）——「校验文件 → 生成启动参数」，
//!   产出 [`PreparedLaunch`]（完整性检查结果 + 生成好的启动参数）；
//! * [`EnsureGameFilesTask`]（`Task<PreparedLaunch, PreparedLaunch>`）——缺件时跑一次
//!   `dmcl_install` 补全，再把原值传下去；
//! * [`LaunchTask`]（`Task<PreparedLaunch, GameMonitor>`）——用已生成的参数启动子进程，
//!   产出**未 wait** 的 [`GameMonitor`]。
//!
//! [`launch_chain`] 把三者串成 `TaskChain<(), GameMonitor>`：由调用者运行、订阅事件，
//! 并在拿到未 wait 的 [`GameMonitor`] 后决定是等待退出还是中途停掉。

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use dmcl_install::InstallOptions;
use dmcl_task::{BoxFuture, Task, TaskChain, TaskContext};

use crate::integrity::IntegrityStatus;
use crate::launcher::{command_preview, spawn_game, GameLauncher};
use crate::monitor::GameMonitor;
use crate::option::LaunchOption;

/// 「校验 → 生成参数」的产出。
#[derive(Debug, Clone)]
pub struct PreparedLaunch {
    /// 完整性检查结果（完整吗、缺哪些文件）。
    pub integrity: IntegrityStatus,
    /// 生成好的启动参数；第一个元素是 java 可执行文件。
    pub args: Vec<String>,
    /// 转义后的命令行，用于展示 / 交给别人执行。
    pub command: String,
    /// 游戏进程的工作目录（`versions/<name>`）。
    pub version_path: PathBuf,
}

/// 把「校验文件 → 生成启动参数」打包成一个任务。
pub struct PrepareLaunchTask {
    option: LaunchOption,
}

impl PrepareLaunchTask {
    pub fn new(option: LaunchOption) -> Self {
        Self { option }
    }
}

impl Task<(), PreparedLaunch> for PrepareLaunchTask {
    fn name(&self) -> &str {
        "准备启动"
    }

    fn run<'a>(&'a self, _input: (), ctx: TaskContext) -> BoxFuture<'a, Result<PreparedLaunch>> {
        let option = self.option.clone();
        Box::pin(async move {
            let mut launcher = GameLauncher::new(option).await?;

            ctx.log("检查文件完整性");
            let integrity = launcher.check_integrity().await?;
            ctx.log(integrity.summary());

            ctx.log("生成启动参数");
            let args = launcher.gen_args_in(&ctx).await?;
            let command = shell_words::join(&args);

            ctx.log("启动参数已就绪");
            Ok(PreparedLaunch {
                integrity,
                args,
                command,
                version_path: launcher.version_path.clone(),
            })
        })
    }
}

/// 缺件时自动补全游戏文件的任务：`Task<PreparedLaunch, PreparedLaunch>`。
///
/// 完整性检查发现缺失文件时，跑一次 [`dmcl_install::game_files_task`] 把文件铺好，
/// 再把**原样的** [`PreparedLaunch`] 传给下一步；已经完整时直接透传（只在进度里
/// 留一个步骤）。
pub struct EnsureGameFilesTask {
    install: Arc<InstallOptions>,
    version_name: String,
}

impl EnsureGameFilesTask {
    pub fn new(install: Arc<InstallOptions>, version_name: impl Into<String>) -> Self {
        Self {
            install,
            version_name: version_name.into(),
        }
    }
}

impl Task<PreparedLaunch, PreparedLaunch> for EnsureGameFilesTask {
    fn name(&self) -> &str {
        "补全游戏文件"
    }

    fn run<'a>(
        &'a self,
        input: PreparedLaunch,
        ctx: TaskContext,
    ) -> BoxFuture<'a, Result<PreparedLaunch>> {
        Box::pin(async move {
            if input.integrity.is_complete() {
                return Ok(input);
            }

            // 缺件摘要与明细：一条条作为日志上报，由前端逐行显示
            ctx.log(input.integrity.summary());
            for line in input.integrity.detail(10).lines() {
                ctx.log(line);
            }

            // 用父 ctx 跑下载链，进度会转发到当前事件流
            let chain =
                dmcl_install::game_files_task(self.install.clone(), self.version_name.clone());
            Task::<(), ()>::run(&chain, (), ctx).await?;
            Ok(input)
        })
    }
}

/// 用 [`PreparedLaunch`] 里已经生成好的参数启动游戏，产出**未 wait** 的
/// [`GameMonitor`]（等待 / 结束由调用者决定）。
pub struct LaunchTask;

impl Task<PreparedLaunch, GameMonitor> for LaunchTask {
    fn name(&self) -> &str {
        "启动游戏"
    }

    fn run<'a>(
        &'a self,
        input: PreparedLaunch,
        ctx: TaskContext,
    ) -> BoxFuture<'a, Result<GameMonitor>> {
        Box::pin(async move {
            // 命令行经由事件流显示（走进度打印机），避免直接 println 与进度行抢行
            ctx.log(format!(
                "命令：{}",
                command_preview(&input.version_path, &input.args)
            ));
            spawn_game(&input.version_path, &input.args)
        })
    }
}

/// 组装「准备 → 补全 → 启动」三段链，返回 `TaskChain<(), GameMonitor>`（不运行）。
pub fn launch_chain(
    option: LaunchOption,
    install: Arc<InstallOptions>,
) -> TaskChain<(), GameMonitor> {
    let version_name = option.version_name.clone();
    TaskChain::new("启动")
        .then(PrepareLaunchTask::new(option), "准备启动")
        .then(
            EnsureGameFilesTask::new(install, version_name),
            "补全游戏文件",
        )
        .then(LaunchTask, "启动游戏")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::OfflineAccount;
    use crate::java::{JavaChoice, JavaConfigs, JavaOption};
    use std::sync::Arc;

    #[test]
    fn the_task_reports_its_name() {
        let option = LaunchOption::new(Arc::new(OfflineAccount::new("p")), "/mc", "1.0");
        let task = PrepareLaunchTask::new(option);
        assert_eq!(task.name(), "准备启动");
    }

    #[test]
    fn the_launch_chain_is_prepare_repair_then_launch() {
        let option = LaunchOption::new(Arc::new(OfflineAccount::new("p")), "/mc", "1.0");
        let install = Arc::new(InstallOptions::new(
            "/mc",
            dmcl_mcmeta::source::DownloadSource::official(),
        ));
        let chain = launch_chain(option, install);
        let names: Vec<_> = chain.step_names().collect();
        assert_eq!(names, ["准备启动", "补全游戏文件", "启动游戏"]);
    }

    /// 找一个能用的 java 可执行文件；本机没有就返回 `None`（跳过测试）。
    async fn any_java() -> Option<PathBuf> {
        for candidate in [
            "/usr/bin/java",
            "/lib/jvm/java-27-openjdk/bin/java",
            "/lib/jvm/java-8-openjdk/jre/bin/java",
        ] {
            let path = PathBuf::from(candidate);
            if JavaOption::probe(path.clone())
                .await
                .map(|java| java.is_usable())
                .unwrap_or(false)
            {
                return Some(path);
            }
        }
        None
    }

    /// 完整跑一遍：校验（空版本即完整）→ 生成参数 → 直接拿到产出。
    #[tokio::test]
    async fn preparing_produces_the_launch_args_and_integrity() {
        let Some(java) = any_java().await else {
            return; // 本机没有 java，跳过
        };

        let root = std::env::temp_dir().join(format!("dmcl-prepare-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&root).await;

        // 一个最小版本：没有库、没有资源，因此完整性检查"完整"
        let json = serde_json::json!({
            "id": "1.0",
            "mainClass": "com.example.Main",
            "arguments": {"jvm": [], "game": ["--demo"]}
        });
        let path = dmcl_mcmeta::layout::version_json_path(&root, "1.0");
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&path, serde_json::to_string(&json).unwrap())
            .await
            .unwrap();

        let mut configs = JavaConfigs::new();
        configs.insert("test".to_string(), java);
        let option =
            LaunchOption::new(Arc::new(OfflineAccount::new("player")), root.clone(), "1.0")
                .with_java(JavaChoice::Named("test".to_string()), configs);

        let task = PrepareLaunchTask::new(option);
        // 结果直接由 run 返回，不再需要共享槽
        let prepared = task
            .run((), TaskContext::detached("准备启动"))
            .await
            .unwrap();

        assert!(
            prepared.integrity.is_complete(),
            "{:?}",
            prepared.integrity.missing
        );
        assert!(prepared.args[0].contains("java"), "{}", prepared.args[0]);
        assert!(
            prepared.args.iter().any(|arg| arg == "com.example.Main"),
            "{:?}",
            prepared.args
        );
        assert!(prepared.args.iter().any(|arg| arg == "--demo"));
        assert!(!prepared.command.is_empty());
        assert_eq!(prepared.version_path, root.join("versions").join("1.0"));

        tokio::fs::remove_dir_all(&root).await.unwrap();
    }
}
