//! 启动流程：检查并补全 → 生成参数 → 启动。
//!
//! * [`PrepareLaunchFilesTask`]（`Task<(), IntegrityStatus>`）——「检查文件完整性；
//!   缺件时跑一次 `dmcl-install` 补全」，产出完整性检查结果；
//! * [`GenerateArgsTask`]（`Task<IntegrityStatus, PreparedLaunch>`）——文件齐了之后再
//!   生成启动参数，产出 [`PreparedLaunch`]；
//! * [`LaunchTask`]（`Task<PreparedLaunch, GameMonitor>`）——用已生成的参数启动子进程，
//!   产出**未 wait** 的 [`GameMonitor`]。
//!
//! [`launch_chain`] 把三者串成 `TaskChain<(), GameMonitor>`：由调用者运行、订阅事件，
//! 并在拿到未 wait 的 [`GameMonitor`] 后决定是等待退出还是中途停掉。
//!
//! 「先补全文件、再生成参数」是刻意的顺序：参数只在文件齐备后才拼，避免在缺件时
//! 生成一份用不了（或需要重拼）的命令行。

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use dmcl_install::{InstallOptions, IntegrityStatus};
use dmcl_task::{BoxFuture, Task, TaskChain, TaskContext};

use crate::launcher::{command_preview, spawn_game, GameLauncher};
use crate::monitor::GameMonitor;
use crate::option::LaunchOption;

/// 「补全文件 → 生成参数」之后的产出。
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

/// 检查文件完整性；缺件时跑一次 `dmcl-install` 补全。
///
/// 完整性只检查一次：结果既用来判断要不要补全，也原样传给下一步。
pub struct PrepareLaunchFilesTask {
    option: LaunchOption,
    install: Arc<InstallOptions>,
    version_name: String,
}

impl PrepareLaunchFilesTask {
    pub fn new(option: LaunchOption, install: Arc<InstallOptions>) -> Self {
        let version_name = option.version_name.clone();
        Self {
            option,
            install,
            version_name,
        }
    }
}

impl Task<(), IntegrityStatus> for PrepareLaunchFilesTask {
    fn name(&self) -> &str {
        "检查并补全文件"
    }

    fn run<'a>(&'a self, _input: (), ctx: TaskContext) -> BoxFuture<'a, Result<IntegrityStatus>> {
        Box::pin(async move {
            let launcher = GameLauncher::new(self.option.clone()).await?;
            let integrity = launcher.check_integrity().await?;
            ctx.log(integrity.summary());

            // 缺件时才跑下载链（链内会跳过已存在的文件），进度转发到当前事件流
            if !integrity.is_complete() {
                let chain =
                    dmcl_install::game_files_task(self.install.clone(), self.version_name.clone());
                Task::<(), ()>::run(&chain, (), ctx).await?;
            }
            Ok(integrity)
        })
    }
}

/// 文件齐备后生成启动参数。
pub struct GenerateArgsTask {
    option: LaunchOption,
}

impl GenerateArgsTask {
    pub fn new(option: LaunchOption) -> Self {
        Self { option }
    }
}

impl Task<IntegrityStatus, PreparedLaunch> for GenerateArgsTask {
    fn name(&self) -> &str {
        "生成启动参数"
    }

    fn run<'a>(
        &'a self,
        integrity: IntegrityStatus,
        ctx: TaskContext,
    ) -> BoxFuture<'a, Result<PreparedLaunch>> {
        Box::pin(async move {
            let mut launcher = GameLauncher::new(self.option.clone()).await?;
            let args = launcher.gen_args_in(&ctx).await?;
            let command = command_preview(&launcher.version_path, &args);
            Ok(PreparedLaunch {
                integrity,
                args,
                command,
                version_path: launcher.version_path.clone(),
            })
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

/// 组装「检查并补全 → 生成参数 → 启动」三段链，返回 `TaskChain<(), GameMonitor>`（不运行）。
pub fn launch_chain(
    option: LaunchOption,
    install: Arc<InstallOptions>,
) -> TaskChain<(), GameMonitor> {
    TaskChain::new("启动")
        .then(
            PrepareLaunchFilesTask::new(option.clone(), install),
            "检查并补全文件",
        )
        .then(GenerateArgsTask::new(option), "生成启动参数")
        .then(LaunchTask, "启动游戏")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::OfflineAccount;
    use crate::java::{JavaChoice, JavaConfigs, JavaOption};
    use dmcl_mcmeta::source::DownloadSource;
    use std::sync::Arc;

    fn install_options(root: &std::path::Path) -> Arc<InstallOptions> {
        Arc::new(InstallOptions::new(root, DownloadSource::official()))
    }

    #[test]
    fn the_tasks_report_their_names() {
        let option = LaunchOption::new(Arc::new(OfflineAccount::new("p")), "/mc", "1.0");
        let install = install_options(std::path::Path::new("/mc"));
        assert_eq!(
            PrepareLaunchFilesTask::new(option.clone(), install).name(),
            "检查并补全文件"
        );
        assert_eq!(GenerateArgsTask::new(option).name(), "生成启动参数");
    }

    #[test]
    fn the_launch_chain_checks_fills_generates_then_launches() {
        let option = LaunchOption::new(Arc::new(OfflineAccount::new("p")), "/mc", "1.0");
        let install = install_options(std::path::Path::new("/mc"));
        let chain = launch_chain(option, install);
        let names: Vec<_> = chain.step_names().collect();
        assert_eq!(names, ["检查并补全文件", "生成启动参数", "启动游戏"]);
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

    /// 完整跑一遍：检查（空版本即完整）→ 生成参数 → 直接拿到产出。
    #[tokio::test]
    async fn checking_then_generating_produces_the_launch_args_and_integrity() {
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

        let integrity = PrepareLaunchFilesTask::new(option.clone(), install_options(&root))
            .run((), TaskContext::detached("检查并补全文件"))
            .await
            .unwrap();
        assert!(integrity.is_complete(), "{:?}", integrity.missing);

        let prepared = GenerateArgsTask::new(option)
            .run(integrity, TaskContext::detached("生成启动参数"))
            .await
            .unwrap();

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
