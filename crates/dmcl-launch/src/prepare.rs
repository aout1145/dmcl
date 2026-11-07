//! 启动流程：读取版本 → 检查并补全 / 选择 Java → 解压 natives → 登录 → 生成参数 → 启动。
//!
//! * [`ReadVersionTask`]（`Task<(), ResolvedVersion>`）——读取版本 JSON 并解析
//!   `inheritsFrom`；
//! * [`PrepareLaunchFilesTask`]（`Task<Arc<ResolvedVersion>, ResolvedVersion>`）——检查文件
//!   完整性，缺件时补全；
//! * [`SelectJavaTask`]（`Task<Arc<ResolvedVersion>, JavaOption>`）——读取版本要求的 Java
//!   大版本，通过解析器选好 java；允许自动安装时下载 JRE；版本 / 架构不匹配时先等用户确认；
//! * [`PrepareNativesTask`]（`Task<(Arc<ResolvedVersion>, Arc<JavaOption>), JavaOption>`）——
//!   按当前平台的 os 与已选 java 的 arch 下载 + 解压 natives；
//! * [`LoginTask`]（`Task<(Arc<ResolvedVersion>, Arc<JavaOption>), Account>`）——等文件补全和
//!   natives 都完成后，让账户来源产出一个可用会话，失败即中止；
//! * [`GenerateArgsTask`]（`Task<(Arc<Account>, Arc<JavaOption>), PreparedLaunch>`）——用已经
//!   选好的 java 生成启动参数，产出 [`PreparedLaunch`]；
//! * [`LaunchTask`]（`Task<Arc<PreparedLaunch>, GameMonitor>`）——用已生成的参数启动子进程，
//!   产出**未 wait** 的 [`GameMonitor`]。
//!
//! [`launch_chain`] 把它们组成 `TaskGraph<GameMonitor>`，显示为「检查游戏完整性 → 登录 →
//! 启动」三步：读取版本后，检查文件与选 Java 并行；natives 依赖两者；登录等文件补全和
//! natives 都结束才开始，避免会话在长时间下载中过期；生成参数依赖账户与选好的 Java，
//! 启动是唯一的最终节点。

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use dmcl_download::DownloadContext;
use dmcl_game::{
    check_integrity, ensure_files, ensure_natives, resolve_version, GameFilesOptions,
    ResolvedVersion, VersionRef,
};
use dmcl_java::{
    install_jre, JavaDetails, JavaOption, JavaRuntimeOptions, JreAutoInstall, JreInstallRequest,
};
use dmcl_mcmeta::os::{Arch, Os};
use dmcl_task::{Task, TaskContext, TaskGraph, TaskGraphBuilder};

use crate::account::{Account, AccountSource};
use crate::launcher::{command_preview, spawn_game, GameLauncher};
use crate::monitor::GameMonitor;
use crate::option::LaunchOption;

/// 生成启动参数之后的产出。
#[derive(Debug, Clone)]
pub struct PreparedLaunch {
    /// 生成好的启动参数；第一个元素是 java 可执行文件。
    pub args: Vec<String>,
    /// 转义后的命令行，用于展示 / 交给别人执行。
    pub command: String,
    /// 游戏进程的工作目录（开启版本隔离时是 `versions/<name>`，否则是根目录）。
    pub game_path: PathBuf,
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

/// 检查文件完整性；缺件时补全。
pub struct PrepareLaunchFilesTask {
    game: Arc<GameFilesOptions>,
    downloads: Arc<DownloadContext>,
}

impl PrepareLaunchFilesTask {
    pub fn new(game: Arc<GameFilesOptions>, downloads: Arc<DownloadContext>) -> Self {
        Self { game, downloads }
    }
}

#[async_trait::async_trait]
impl Task<Arc<ResolvedVersion>, ResolvedVersion> for PrepareLaunchFilesTask {
    async fn run(
        &self,
        resolved: Arc<ResolvedVersion>,
        ctx: TaskContext,
    ) -> Result<ResolvedVersion> {
        let integrity = check_integrity(&resolved, &self.game).await?;
        ctx.log(integrity.summary());
        if !integrity.is_complete() {
            ensure_files(&resolved, &self.game, &self.downloads, &ctx).await?;
        }
        Ok((*resolved).clone())
    }
}

/// 通过解析器选好 java；允许自动安装时下载匹配的 JRE；版本 / 架构仍不匹配时先等用户确认。
pub struct SelectJavaTask {
    option: LaunchOption,
    game: Arc<GameFilesOptions>,
    downloads: Arc<DownloadContext>,
    jre: Option<JreAutoInstall>,
}

impl SelectJavaTask {
    pub fn new(
        option: LaunchOption,
        game: Arc<GameFilesOptions>,
        downloads: Arc<DownloadContext>,
        jre: Option<JreAutoInstall>,
    ) -> Self {
        Self {
            option,
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
        if let Some(hook) = &self.option.java_installed_hook {
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

        let mut downloaded = false;
        let mut java = match self.option.java.resolve(required_major).await {
            Ok(java) => java,
            Err(err) => {
                let Some(jre) = &self.jre else {
                    return Err(err);
                };
                ctx.log(format!("没有找到合适的 Java：{err:#}，尝试自动下载"));
                downloaded = true;
                self.auto_install_java(preferred.as_deref(), required_major, jre, &ctx)
                    .await?
            }
        };

        if !downloaded && !java_is_suitable(&java, required_major) {
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

/// 按当前平台的 os 与已选 java 的 arch 下载 + 解压 natives。
///
/// natives 的分类器（`natives-linux-64` 之类）取决于运行游戏的 JVM 架构，所以必须用
/// 上一步选好的 java；这里不再选择 Java。
pub struct PrepareNativesTask {
    game: Arc<GameFilesOptions>,
    downloads: Arc<DownloadContext>,
}

impl PrepareNativesTask {
    pub fn new(game: Arc<GameFilesOptions>, downloads: Arc<DownloadContext>) -> Self {
        Self { game, downloads }
    }
}

#[async_trait::async_trait]
impl Task<(Arc<ResolvedVersion>, Arc<JavaOption>), JavaOption> for PrepareNativesTask {
    async fn run(
        &self,
        (version, java): (Arc<ResolvedVersion>, Arc<JavaOption>),
        ctx: TaskContext,
    ) -> Result<JavaOption> {
        ensure_natives(
            &version,
            Os::current(),
            java.arch,
            &self.game,
            &self.downloads,
            &ctx,
        )
        .await?;
        Ok((*java).clone())
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

#[async_trait::async_trait]
impl Task<(Arc<Account>, Arc<JavaOption>), PreparedLaunch> for GenerateArgsTask {
    async fn run(
        &self,
        (account, java): (Arc<Account>, Arc<JavaOption>),
        ctx: TaskContext,
    ) -> Result<PreparedLaunch> {
        let mut launcher = GameLauncher::new(self.option.clone(), (*account).clone()).await?;
        // 复用上一步选好的 java，避免再选一次
        launcher.java = Some((*java).clone());
        let args = launcher.gen_args(&ctx).await?;
        let command = command_preview(&launcher.game_path, &args);
        Ok(PreparedLaunch {
            args,
            command,
            game_path: launcher.game_path.clone(),
        })
    }
}

/// 用 [`PreparedLaunch`] 里已经生成好的参数启动游戏，产出**未 wait** 的
/// [`GameMonitor`]（等待 / 结束由调用者决定）。
pub struct LaunchTask;

#[async_trait::async_trait]
impl Task<Arc<PreparedLaunch>, GameMonitor> for LaunchTask {
    async fn run(&self, input: Arc<PreparedLaunch>, ctx: TaskContext) -> Result<GameMonitor> {
        // 命令行经由事件流显示（走进度打印机），避免直接 println 与进度行抢行
        ctx.log(format!(
            "命令：{}",
            command_preview(&input.game_path, &input.args)
        ));
        spawn_game(&input.game_path, &input.args)
    }
}

/// 组装「检查游戏完整性 → 登录 → 启动」任务图。
///
/// 检查游戏完整性里，读取版本后检查文件与选 Java 并行，natives 同时依赖版本与 Java；
/// 登录等文件补全和 natives 都完成；启动里生成参数同时依赖账户与选好的 java，最终节点
/// 「启动游戏」返回 `GameMonitor`。
pub fn launch_chain(
    option: LaunchOption,
    game: Arc<GameFilesOptions>,
    jre: Option<JreAutoInstall>,
) -> TaskGraph<GameMonitor> {
    let version = option.version.clone();
    let account_source = option.account_source.clone();

    let mut builder = TaskGraphBuilder::new("启动");
    let downloads = game.download_context(builder.progress());
    let integrity = builder.step("检查游戏完整性");

    let read = builder.add(integrity, "读取版本", (), ReadVersionTask::new(version));
    let files = builder.add(
        integrity,
        "检查并补全文件",
        read.clone(),
        PrepareLaunchFilesTask::new(game.clone(), downloads.clone()),
    );
    let java = builder.add(
        integrity,
        "选择 Java",
        read.clone(),
        SelectJavaTask::new(option.clone(), game.clone(), downloads.clone(), jre),
    );
    let natives = builder.add(
        integrity,
        "解压 natives",
        (read, java.clone()),
        PrepareNativesTask::new(game, downloads),
    );

    let login_step = builder.step("登录");
    let account = builder.add(
        login_step,
        "登录",
        (files, natives),
        LoginTask::new(account_source),
    );

    let launch_step = builder.step("启动");
    let prepared = builder.add(
        launch_step,
        "生成启动参数",
        (account, java),
        GenerateArgsTask::new(option),
    );
    let sink = builder.add(launch_step, "启动游戏", prepared, LaunchTask);
    builder.build(sink).expect("启动图的最终节点由构造保证")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::OfflineAccountSource;
    use dmcl_game::GameFilesOptions;
    use dmcl_java::{default_java_roots, find_java};
    use dmcl_mcmeta::source::DownloadSource;
    use dmcl_task::{task_fn, ProgressHandle, TaskEvent, TaskGraphBuilder};
    use std::sync::Arc;

    fn game_options() -> Arc<GameFilesOptions> {
        Arc::new(GameFilesOptions::new(DownloadSource::official()))
    }

    fn downloads(game: &GameFilesOptions) -> Arc<DownloadContext> {
        game.download_context(ProgressHandle::new())
    }

    fn offline_source() -> Arc<dyn AccountSource> {
        Arc::new(OfflineAccountSource::new("p"))
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

        let root = std::env::temp_dir().join(format!("dmcl-prepare-{}", std::process::id()));
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

        let option = LaunchOption::new(
            Arc::new(OfflineAccountSource::new("player")),
            VersionRef::new(&root, "1.0"),
            Arc::new(java),
            None,
        );
        let game = game_options();
        let downloads = downloads(&game);

        let version = VersionRef::new(&root, "1.0");
        let resolved = run_node(move |ctx| {
            let task = ReadVersionTask::new(version.clone());
            async move { task.run((), ctx).await }
        })
        .await
        .unwrap();
        assert_eq!(resolved.json.id.as_deref(), Some("1.0"));

        let (game_for_task, downloads_for_task) = (game.clone(), downloads.clone());
        let resolved_for_task = resolved.clone();
        let files = run_node(move |ctx| {
            let task =
                PrepareLaunchFilesTask::new(game_for_task.clone(), downloads_for_task.clone());
            let resolved = resolved_for_task.clone();
            async move { task.run(resolved, ctx).await }
        })
        .await
        .unwrap();
        assert_eq!(files.json.id.as_deref(), Some("1.0"));

        let (option_for_task, game_for_task, downloads_for_task) =
            (option.clone(), game.clone(), downloads.clone());
        let resolved_for_task = resolved.clone();
        let java = run_node(move |ctx| {
            let task = SelectJavaTask::new(
                option_for_task.clone(),
                game_for_task.clone(),
                downloads_for_task.clone(),
                None,
            );
            let resolved = resolved_for_task.clone();
            async move { task.run(resolved, ctx).await }
        })
        .await
        .unwrap();
        assert!(java.major > 0);

        let (game_for_task, downloads_for_task) = (game.clone(), downloads.clone());
        let resolved_for_task = resolved.clone();
        let java_for_task = java.clone();
        let java = run_node(move |ctx| {
            let task = PrepareNativesTask::new(game_for_task.clone(), downloads_for_task.clone());
            let input = (resolved_for_task.clone(), java_for_task.clone());
            async move { task.run(input, ctx).await }
        })
        .await
        .unwrap();
        assert!(java.major > 0);

        let source = option.account_source.clone();
        let resolved_for_task = resolved.clone();
        let java_for_task = java.clone();
        let account = run_node(move |ctx| {
            let task = LoginTask::new(source.clone());
            let input = (resolved_for_task.clone(), java_for_task.clone());
            async move { task.run(input, ctx).await }
        })
        .await
        .unwrap();
        assert_eq!(account.username, "player");

        let account_for_task = account.clone();
        let java_for_task = java.clone();
        let prepared = run_node(move |ctx| {
            let task = GenerateArgsTask::new(option.clone());
            let input = (account_for_task.clone(), java_for_task.clone());
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
        assert!(!prepared.command.is_empty());
        assert_eq!(prepared.game_path, root.join("versions").join("1.0"));

        tokio::fs::remove_dir_all(&root).await.unwrap();
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
        let option = LaunchOption::new(
            offline_source(),
            VersionRef::new(&root, "1.0"),
            Arc::new(JavaOption {
                path: PathBuf::from("/opt/jdk8/bin/java"),
                major: 8,
                arch,
            }),
            None,
        );
        let resolved = resolve_version(&VersionRef::new(&root, "1.0"))
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
            SelectJavaTask::new(option, game, downloads, None),
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
