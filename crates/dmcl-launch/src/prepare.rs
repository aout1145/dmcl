//! 「校验文件 → 生成启动参数」作为一个任务。
//!
//! [`PrepareLaunchTask`] 是 `Task<(), PreparedLaunch>`：接收空输入、产出
//! [`PreparedLaunch`]（完整性检查结果 + 生成好的启动参数）。直接 `run` 就能拿到
//! 结果，也可以塞进任务链把结果继续往下传。

use std::path::PathBuf;

use anyhow::Result;
use dmcl_task::{BoxFuture, Task, TaskContext};

use crate::integrity::IntegrityStatus;
use crate::launcher::GameLauncher;
use crate::option::LaunchOption;
use crate::util::escape_shell_command;

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

            ctx.status("检查文件完整性");
            let integrity = launcher.check_integrity().await?;
            ctx.status(integrity.summary());

            ctx.status("生成启动参数");
            let args = launcher.gen_args().await?;
            let command = escape_shell_command(&args);

            ctx.status("启动参数已就绪");
            Ok(PreparedLaunch {
                integrity,
                args,
                command,
                version_path: launcher.version_path.clone(),
            })
        })
    }
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
        let path = dmcl_mcmeta::version_json_path(&root, "1.0");
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
        let prepared = task.run((), TaskContext::detached("准备启动")).await.unwrap();

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
