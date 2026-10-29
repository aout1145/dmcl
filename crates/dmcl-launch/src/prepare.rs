//! 启动流程：登录 → 检查并补全 → 准备 natives → 生成参数 → 启动。
//!
//! * [`LoginTask`]（`Task<(), Account>`）——让账户来源产出一个可用会话，失败即中止；
//! * [`PrepareLaunchFilesTask`]（`Task<Account, (Account, IntegrityStatus)>`）——「检查文件
//!   完整性；缺件时跑一次 `dmcl-install` 补全」，产出完整性检查结果；
//! * [`PrepareNativesTask`]（`Task<(Account, IntegrityStatus), (Account, JavaOption)>`）——
//!   先通过选择器选好 java，再按当前平台的 os 与 java 的 arch 下载 + 解压 natives
//!   （natives 只有在选好 java 后才能确定）；
//! * [`GenerateArgsTask`]（`Task<(Account, JavaOption), PreparedLaunch>`）——用已经选好的
//!   java 生成启动参数，产出 [`PreparedLaunch`]；
//! * [`LaunchTask`]（`Task<PreparedLaunch, GameMonitor>`）——用已生成的参数启动子进程，
//!   产出**未 wait** 的 [`GameMonitor`]。
//!
//! [`launch_chain`] 把五者串成 `TaskChain<(), GameMonitor>`：由调用者运行、订阅事件，
//! 并在拿到未 wait 的 [`GameMonitor`] 后决定是等待退出还是中途停掉。
//!
//! 「先登录、再补全文件、最后生成参数」是刻意的顺序：认证失败时立即中止，不会开始下载；
//! 参数只在文件齐备后才拼，避免在缺件时生成一份用不了（或需要重拼）的命令行。

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use dmcl_install::{InstallOptions, IntegrityStatus};
use dmcl_mcmeta::layout::read_version_json_safe;
use dmcl_mcmeta::os::Os;
use dmcl_task::{BoxFuture, Task, TaskChain, TaskContext};

use crate::account::{Account, AccountSource};
use crate::java::JavaOption;
use crate::launcher::{command_preview, spawn_game, GameLauncher};
use crate::monitor::GameMonitor;
use crate::option::LaunchOption;

/// 登录之后沿链传下去的账户快照与上一步产物。
type WithAccount<T> = (Account, T);

/// 「补全文件 → 准备 natives → 生成参数」之后的产出。
#[derive(Debug, Clone)]
pub struct PreparedLaunch {
    /// 生成好的启动参数；第一个元素是 java 可执行文件。
    pub args: Vec<String>,
    /// 转义后的命令行，用于展示 / 交给别人执行。
    pub command: String,
    /// 游戏进程的工作目录（`versions/<name>`）。
    pub version_path: PathBuf,
}

/// 让账户来源产出一个可用会话。
///
/// 离线账户直接通过；在线实现以后可以在这里静默刷新或走完整登录。登录失败会中止
/// 整条链，后面的下载 / 启动都不会开始。
pub struct LoginTask {
    account_source: Arc<dyn AccountSource>,
}

impl LoginTask {
    pub fn new(account_source: Arc<dyn AccountSource>) -> Self {
        Self { account_source }
    }
}

impl Task<(), Account> for LoginTask {
    fn run<'a>(&'a self, _input: (), ctx: TaskContext) -> BoxFuture<'a, Result<Account>> {
        Box::pin(async move {
            let account = self.account_source.login().await?;
            ctx.log(format!(
                "账户：{}（{}）",
                account.username, account.user_type
            ));
            Ok(account)
        })
    }
}

/// 检查文件完整性；缺件时跑一次 `dmcl-install` 补全。
///
/// 完整性只检查一次：结果既用来判断要不要补全，也原样传给下一步。
pub struct PrepareLaunchFilesTask {
    install: Arc<InstallOptions>,
    version_name: String,
}

impl PrepareLaunchFilesTask {
    pub fn new(install: Arc<InstallOptions>, version_name: impl Into<String>) -> Self {
        Self {
            install,
            version_name: version_name.into(),
        }
    }
}

impl Task<Account, WithAccount<IntegrityStatus>> for PrepareLaunchFilesTask {
    fn run<'a>(
        &'a self,
        account: Account,
        ctx: TaskContext,
    ) -> BoxFuture<'a, Result<WithAccount<IntegrityStatus>>> {
        Box::pin(async move {
            let integrity =
                dmcl_install::check_integrity(&self.install.plan(), &self.version_name).await?;
            ctx.log(integrity.summary());

            // 缺件时才跑下载链（链内会跳过已存在的文件），进度转发到当前事件流
            if !integrity.is_complete() {
                let chain =
                    dmcl_install::game_files_task(self.install.clone(), self.version_name.clone());
                Task::<(), ()>::run(&chain, (), ctx).await?;
            }
            Ok((account, integrity))
        })
    }
}

/// 通过选择器选好 java，并按当前平台的 os 与 java 的 arch 下载 + 解压 natives。
///
/// natives 的分类器（`natives-linux-64` 之类）取决于运行游戏的 JVM 架构，所以必须在
/// 选好 java 之后才能确定要下哪一个、解到哪个目录。
pub struct PrepareNativesTask {
    option: LaunchOption,
    install: Arc<InstallOptions>,
}

impl PrepareNativesTask {
    pub fn new(option: LaunchOption, install: Arc<InstallOptions>) -> Self {
        Self { option, install }
    }
}

impl Task<WithAccount<IntegrityStatus>, WithAccount<JavaOption>> for PrepareNativesTask {
    fn run<'a>(
        &'a self,
        (account, _integrity): WithAccount<IntegrityStatus>,
        ctx: TaskContext,
    ) -> BoxFuture<'a, Result<WithAccount<JavaOption>>> {
        Box::pin(async move {
            let json =
                read_version_json_safe(&self.option.root_path, &self.option.version_name).await?;
            let java = self
                .option
                .java
                .select_java(json.required_java_major())
                .await?;
            ctx.log(format!(
                "使用 Java {}：{}（{} / {}）",
                java.major,
                java.path.display(),
                Os::current().mc_name(),
                java.arch.name()
            ));

            let chain = dmcl_install::natives_task(
                self.install.clone(),
                self.option.version_name.clone(),
                Os::current(),
                java.arch,
            );
            Task::<(), ()>::run(&chain, (), ctx).await?;
            Ok((account, java))
        })
    }
}

/// 用已经选好的 java 生成启动参数。
pub struct GenerateArgsTask {
    option: LaunchOption,
}

impl GenerateArgsTask {
    pub fn new(option: LaunchOption) -> Self {
        Self { option }
    }
}

impl Task<WithAccount<JavaOption>, PreparedLaunch> for GenerateArgsTask {
    fn run<'a>(
        &'a self,
        (account, java): WithAccount<JavaOption>,
        ctx: TaskContext,
    ) -> BoxFuture<'a, Result<PreparedLaunch>> {
        Box::pin(async move {
            let mut launcher = GameLauncher::new(self.option.clone(), account).await?;
            // 复用上一步选好的 java，避免再选一次
            launcher.java = Some(java);
            let args = launcher.gen_args_in(&ctx).await?;
            let command = command_preview(&launcher.version_path, &args);
            Ok(PreparedLaunch {
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

/// 组装「登录 → 检查并补全 → 准备 natives → 生成参数 → 启动」五段链，返回
/// `TaskChain<(), GameMonitor>`（不运行）。
pub fn launch_chain(
    option: LaunchOption,
    install: Arc<InstallOptions>,
) -> TaskChain<(), GameMonitor> {
    let version_name = option.version_name.clone();
    let account_source = option.account_source.clone();
    TaskChain::new("启动")
        .then(LoginTask::new(account_source), "登录")
        .then(
            PrepareLaunchFilesTask::new(install.clone(), version_name),
            "检查并补全文件",
        )
        .then(
            PrepareNativesTask::new(option.clone(), install),
            "准备 natives",
        )
        .then(GenerateArgsTask::new(option), "生成启动参数")
        .then(LaunchTask, "启动游戏")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::OfflineAccountSource;
    use crate::java::JavaDetails;
    use dmcl_mcmeta::source::DownloadSource;
    use std::sync::Arc;

    fn install_options(root: &std::path::Path) -> Arc<InstallOptions> {
        Arc::new(InstallOptions::new(root, DownloadSource::official()))
    }

    fn offline_source() -> Arc<dyn AccountSource> {
        Arc::new(OfflineAccountSource::new("p"))
    }

    #[test]
    fn the_launch_chain_logs_in_checks_fills_prepares_generates_then_launches() {
        let option = LaunchOption::new(offline_source(), "/mc", "1.0");
        let install = install_options(std::path::Path::new("/mc"));
        let chain = launch_chain(option, install);
        let names: Vec<_> = chain.step_names().collect();
        assert_eq!(
            names,
            [
                "登录",
                "检查并补全文件",
                "准备 natives",
                "生成启动参数",
                "启动游戏"
            ]
        );
    }

    /// 找一个能用的 java 可执行文件；本机没有就返回 `None`（跳过测试）。
    async fn any_java() -> Option<JavaOption> {
        for candidate in [
            "/usr/bin/java",
            "/lib/jvm/java-27-openjdk/bin/java",
            "/lib/jvm/java-8-openjdk/jre/bin/java",
        ] {
            let path = PathBuf::from(candidate);
            if let Ok(java) = JavaDetails::probe(path).await {
                if java.is_usable() {
                    return Some(java.option);
                }
            }
        }
        None
    }

    /// 完整跑一遍：登录 → 检查（空版本即完整）→ 准备 natives（选 java）→ 生成参数 → 拿到产出。
    #[tokio::test]
    async fn logging_in_then_checking_then_generating_produces_the_launch_args() {
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

        let option = LaunchOption::new(
            Arc::new(OfflineAccountSource::new("player")),
            root.clone(),
            "1.0",
        )
        .with_java(Arc::new(java));

        let account = LoginTask::new(option.account_source.clone())
            .run((), TaskContext::detached("登录"))
            .await
            .unwrap();
        assert_eq!(account.username, "player");

        let (account, integrity) = PrepareLaunchFilesTask::new(install_options(&root), "1.0")
            .run(account, TaskContext::detached("检查并补全文件"))
            .await
            .unwrap();
        assert!(integrity.is_complete(), "{:?}", integrity.missing);

        let (account, java) = PrepareNativesTask::new(option.clone(), install_options(&root))
            .run((account, integrity), TaskContext::detached("准备 natives"))
            .await
            .unwrap();
        assert!(java.major > 0);

        let prepared = GenerateArgsTask::new(option)
            .run((account, java), TaskContext::detached("生成启动参数"))
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
