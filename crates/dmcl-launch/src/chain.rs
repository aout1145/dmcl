//! 启动链：读取版本 → 检查 / 补全文件、选择 Java → 解压 natives → 登录 →
//! 生成参数 → 启动。
//!
//! [`launch_chain`] 把它们组成 `TaskGraph<GameMonitor>`，显示为「检查游戏完整性 → 登录 →
//! 启动」三步：读取版本后，检查文件与选 Java 并行；natives 依赖两者；登录等文件补全和
//! natives 都结束才开始，避免会话在长时间下载中过期；生成参数依赖文件、账户与选好的 Java，
//! 启动是唯一的最终节点。

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use dmcl_game::{resolve_version, GameFilesOptions, ResolvedVersion, VersionRef};
use dmcl_java::{JavaOption, JavaResolver, JreAutoInstall};
use dmcl_task::{Task, TaskContext, TaskGraph, TaskGraphBuilder};

use crate::arguments::{GameLaunchOptions, GenerateArgsTask};
use crate::files::{GameIntegrity, PrepareLaunchFilesTask, PrepareNativesTask};
use crate::java::{JvmCompatibility, SelectJavaTask};
use crate::memory::MemoryOptions;
use crate::monitor::{GameMonitor, LaunchTask, ProcessOptions};
use dmcl_account::{Account, AccountSource};

/// 控制启动链的输入。
///
/// 账户来源与 Java 解析都由调用者通过 `Arc<dyn …>` 传入，结构体因此仍是
/// `Send + Sync`，可以交给 GUI 的另一个线程。
pub struct LaunchChainOptions {
    pub account_source: Arc<dyn AccountSource>,
    /// 要启动的版本。
    pub version: VersionRef,
    /// 怎么选 Java。
    pub java: Arc<dyn JavaResolver>,
    /// 启动时对所选 Java 的兼容性检查。
    pub jvm_compatibility: JvmCompatibility,
    /// 启动时对游戏文件完整性的处理方式。
    pub game_integrity: GameIntegrity,
    /// 用户的内存偏好；由 [`GenerateArgsTask`] 按已选 Java 的架构换算成确定值。
    pub memory: MemoryOptions,
}

impl fmt::Debug for LaunchChainOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LaunchChainOptions")
            .field("account", &self.account_source.name())
            .field("version", &self.version)
            .field("jvm_compatibility", &self.jvm_compatibility)
            .field("game_integrity", &self.game_integrity)
            .field("memory", &self.memory)
            .finish_non_exhaustive()
    }
}

/// 读取版本 JSON 并解析 `inheritsFrom`。
pub struct ReadVersionTask {
    version: VersionRef,
}

impl ReadVersionTask {
    pub fn new(version: VersionRef) -> Self {
        Self { version }
    }
}

#[async_trait::async_trait]
impl Task<(), ResolvedVersion> for ReadVersionTask {
    async fn run(&self, _input: (), _ctx: TaskContext) -> Result<ResolvedVersion> {
        resolve_version(&self.version).await
    }
}

/// 让账户来源产出一个可用会话。
///
/// 离线账户直接通过；在线实现只做静默刷新，refresh token 失效时返回错误。
/// 登录失败会中止整张图，后面的参数生成 / 启动都不会开始。
pub struct LoginTask {
    account_source: Arc<dyn AccountSource>,
}

impl LoginTask {
    pub fn new(account_source: Arc<dyn AccountSource>) -> Self {
        Self { account_source }
    }
}

#[async_trait::async_trait]
impl Task<(Arc<ResolvedVersion>, Arc<JavaOption>), Account> for LoginTask {
    async fn run(
        &self,
        _input: (Arc<ResolvedVersion>, Arc<JavaOption>),
        ctx: TaskContext,
    ) -> Result<Account> {
        let account = self.account_source.login().await?;
        ctx.log(format!(
            "账户：{}（{}）",
            account.username, account.user_type
        ));
        Ok(account)
    }
}

/// 组装「检查游戏完整性 → 登录 → 启动」任务图。
///
/// 检查游戏完整性里，读取版本后检查文件与选 Java 并行，natives 同时依赖版本与 Java；
/// 登录等文件补全和 natives 都完成；启动里生成参数同时依赖文件、账户与选好的 java，
/// 最终节点「启动游戏」返回 `GameMonitor`。
pub fn launch_chain(
    chain: LaunchChainOptions,
    game: GameLaunchOptions,
    game_dir: PathBuf,
    process: ProcessOptions,
    files: Arc<GameFilesOptions>,
    jre: Option<JreAutoInstall>,
) -> TaskGraph<GameMonitor> {
    assert_eq!(
        game_dir, process.working_dir,
        "game_dir 必须与 ProcessOptions.working_dir 一致"
    );

    let version = chain.version;
    let account_source = chain.account_source;
    let integrity = chain.game_integrity;
    let java = chain.java;
    let compatibility = chain.jvm_compatibility;
    let memory = chain.memory;

    let mut builder = TaskGraphBuilder::new("启动");
    let downloads = files.download_context(builder.progress());
    let integrity_step = builder.step("检查游戏完整性");

    let read = builder.add(
        integrity_step,
        "读取版本",
        (),
        ReadVersionTask::new(version),
    );
    let files_name = match integrity {
        GameIntegrity::Ignore => "跳过文件检查",
        GameIntegrity::Check => "检查文件",
        GameIntegrity::Complete => "检查并补全文件",
    };
    let prepared_files = builder.add(
        integrity_step,
        files_name,
        read.clone(),
        PrepareLaunchFilesTask::new(files.clone(), downloads.clone(), integrity),
    );
    let selected_java = builder.add(
        integrity_step,
        "选择 Java",
        read.clone(),
        SelectJavaTask::new(java, compatibility, files.clone(), downloads.clone(), jre),
    );
    let natives = builder.add(
        integrity_step,
        "解压 natives",
        (read, selected_java.clone()),
        PrepareNativesTask::new(files, downloads, integrity),
    );

    let login_step = builder.step("登录");
    let account = builder.add(
        login_step,
        "登录",
        (prepared_files.clone(), natives),
        LoginTask::new(account_source),
    );

    let launch_step = builder.step("启动");
    let prepared = builder.add(
        launch_step,
        "生成启动参数",
        (prepared_files, account, selected_java),
        GenerateArgsTask {
            game,
            game_dir,
            memory,
        },
    );
    let sink = builder.add(launch_step, "启动游戏", prepared, LaunchTask { process });
    builder.build(sink).expect("启动图的最终节点由构造保证")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::monitor::ProcessPriority;
    use dmcl_account::OfflineAccountSource;
    use dmcl_java::{default_java_roots, find_java};
    use dmcl_mcmeta::source::DownloadSource;
    use dmcl_task::{task_fn, ProgressHandle};
    use std::sync::Arc;

    fn game_options() -> Arc<GameFilesOptions> {
        Arc::new(GameFilesOptions::new(DownloadSource::official()))
    }

    fn downloads(game: &GameFilesOptions) -> Arc<dmcl_download::DownloadContext> {
        game.download_context(ProgressHandle::new())
    }

    async fn run_node<O, F, Fut>(f: F) -> anyhow::Result<Arc<O>>
    where
        O: Send + Sync + 'static,
        F: Fn(TaskContext) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = anyhow::Result<O>> + Send + 'static,
    {
        let mut builder = TaskGraphBuilder::new("测试");
        let step = builder.step("测试");
        let sink = builder.add(step, "测试", (), task_fn(move |_: (), ctx| f(ctx)));
        builder.build(sink).unwrap().run(|_| {}).await
    }

    /// 找一个能用的 java 可执行文件；本机没有就返回 `None`（跳过测试）。
    async fn any_java() -> Option<JavaOption> {
        find_java(&default_java_roots())
            .await
            .into_iter()
            .next()
            .map(|details| details.option)
    }

    /// 完整跑一遍：读取版本 → 检查（空版本即完整）→ 选 java → 解压 natives → 登录 →
    /// 生成参数。
    #[tokio::test]
    async fn a_minimal_version_runs_the_full_chain_to_launch_args() {
        let Some(java) = any_java().await else {
            return; // 本机没有 java，跳过
        };

        let root = std::env::temp_dir().join(format!("dmcl-chain-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&root).await;

        // 一个最小版本：没有库、没有资源，因此完整性检查「完整」
        let json = serde_json::json!({
            "id": "1.0",
            "mainClass": "com.example.Main",
            "javaVersion": {"majorVersion": java.major},
            "arguments": {"jvm": [], "game": ["--demo"]}
        });
        let path = dmcl_mcmeta::layout::version_json_path(&root, "1.0");
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&path, serde_json::to_string(&json).unwrap())
            .await
            .unwrap();

        let chain = LaunchChainOptions {
            account_source: Arc::new(OfflineAccountSource::new("player")),
            version: VersionRef::new(&root, "1.0"),
            java: Arc::new(java.clone()),
            jvm_compatibility: JvmCompatibility::Check,
            game_integrity: GameIntegrity::Complete,
            memory: MemoryOptions {
                min: Some(512),
                max: Some(2048),
            },
        };
        let game_files = game_options();
        let downloads = downloads(&game_files);
        let game_dir = chain.version.dir();

        let version = chain.version.clone();
        let resolved = run_node(move |ctx| {
            let task = ReadVersionTask::new(version.clone());
            async move { task.run((), ctx).await }
        })
        .await
        .unwrap();
        assert_eq!(resolved.json.id.as_deref(), Some("1.0"));

        let (game_for_task, downloads_for_task) = (game_files.clone(), downloads.clone());
        let resolved_for_task = resolved.clone();
        let files = run_node(move |ctx| {
            let task = PrepareLaunchFilesTask::new(
                game_for_task.clone(),
                downloads_for_task.clone(),
                GameIntegrity::Complete,
            );
            let resolved = resolved_for_task.clone();
            async move { task.run(resolved, ctx).await }
        })
        .await
        .unwrap();
        assert_eq!(files.json.id.as_deref(), Some("1.0"));

        let (java_for_task, game_for_task, downloads_for_task) =
            (chain.java.clone(), game_files.clone(), downloads.clone());
        let resolved_for_task = resolved.clone();
        let selected = run_node(move |ctx| {
            let task = SelectJavaTask::new(
                java_for_task.clone(),
                JvmCompatibility::Check,
                game_for_task.clone(),
                downloads_for_task.clone(),
                None,
            );
            let resolved = resolved_for_task.clone();
            async move { task.run(resolved, ctx).await }
        })
        .await
        .unwrap();
        assert!(selected.major > 0);

        let (game_for_task, downloads_for_task) = (game_files.clone(), downloads.clone());
        let resolved_for_task = resolved.clone();
        let java_for_task = selected.clone();
        let selected = run_node(move |ctx| {
            let task = PrepareNativesTask::new(
                game_for_task.clone(),
                downloads_for_task.clone(),
                GameIntegrity::Complete,
            );
            let input = (resolved_for_task.clone(), java_for_task.clone());
            async move { task.run(input, ctx).await }
        })
        .await
        .unwrap();
        assert!(selected.major > 0);

        let source = chain.account_source.clone();
        let resolved_for_task = resolved.clone();
        let java_for_task = selected.clone();
        let account = run_node(move |ctx| {
            let task = LoginTask::new(source.clone());
            let input = (resolved_for_task.clone(), java_for_task.clone());
            async move { task.run(input, ctx).await }
        })
        .await
        .unwrap();
        assert_eq!(account.username, "player");

        let task = GenerateArgsTask {
            game: GameLaunchOptions::default(),
            game_dir: game_dir.clone(),
            memory: chain.memory,
        };
        let account_for_task = account.clone();
        let java_for_task = selected.clone();
        let prepared = run_node(move |ctx| {
            let task = task.clone();
            let input = (
                resolved.clone(),
                account_for_task.clone(),
                java_for_task.clone(),
            );
            async move { task.run(input, ctx).await }
        })
        .await
        .unwrap();

        assert!(prepared.args[0].contains("java"), "{}", prepared.args[0]);
        assert!(
            prepared.args.iter().any(|arg| arg == "com.example.Main"),
            "{:?}",
            prepared.args
        );
        assert!(prepared.args.iter().any(|arg| arg == "--demo"));
        assert!(prepared.args.contains(&"-Xms512m".to_string()));
        assert!(prepared.args.contains(&"-Xmx2048m".to_string()));
        assert_eq!(prepared.game_dir, game_dir);
        assert_eq!(prepared.resolved.reference, VersionRef::new(&root, "1.0"));
        assert_eq!(prepared.java.major, selected.major);

        tokio::fs::remove_dir_all(&root).await.unwrap();
    }

    #[test]
    #[should_panic(expected = "game_dir 必须与 ProcessOptions.working_dir 一致")]
    fn launch_chain_requires_game_dir_to_match_the_working_directory() {
        let root = std::env::temp_dir();
        let java = JavaOption {
            path: PathBuf::from("/usr/bin/java"),
            major: 17,
            arch: dmcl_mcmeta::os::Arch::X86_64,
        };
        let chain = LaunchChainOptions {
            account_source: Arc::new(OfflineAccountSource::new("player")),
            version: VersionRef::new(&root, "1.0"),
            java: Arc::new(java),
            jvm_compatibility: JvmCompatibility::Check,
            game_integrity: GameIntegrity::Complete,
            memory: MemoryOptions::default(),
        };
        let process = ProcessOptions {
            working_dir: root.join("versions").join("1.0"),
            priority: ProcessPriority::Normal,
        };
        let _ = launch_chain(
            chain,
            GameLaunchOptions::default(),
            root.join("elsewhere"),
            process,
            game_options(),
            None,
        );
    }
}
