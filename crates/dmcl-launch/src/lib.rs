//! 命令行生成、启动与状态监控。
//!
//! 两条设计要点：
//!
//! 1. **底层启动与文件物化解耦**。[`generate_arguments`] 只负责拼命令行，起进程由
//!    [`LaunchTask`] 完成；文件完整性由 [`dmcl_game::check_integrity`] 负责，**是否修复交给
//!    调用者**。想要「缺了就补」的一站式流程时，用 [`launch_chain`]。
//! 2. **不读全局配置**。Java 的选择由调用者实现 [`dmcl_java::JavaResolver`] 注入；窗口
//!    等生成参数用 [`GameLaunchOptions`] 传入，内存偏好用 [`MemoryOptions`] 传入并由调用者
//!    按已选 Java 的架构换算成 [`MemorySize`]。选项因此分成三层：[`LaunchChainOptions`]
//!    控制启动链，[`GameLaunchOptions`] 控制生成启动参数，[`ProcessOptions`] 控制游戏进程。
//!
//! 一站式启动把「检查游戏完整性 → 登录 → 启动」组成一张任务图（见 [`crate::chain`]）：
//! 检查游戏完整性里依次是读取版本、按配置检查 / 补全文件、选择 Java、解压 natives；启动里是
//! 生成参数与起子进程。[`launch_chain`] 返回 `TaskGraph<GameSession>`，由调用者运行、
//! 订阅事件，并在拿到**未 wait** 的 [`GameMonitor`] 后决定何时等待或停掉；[`GameSession`]
//! 同时带着本次启动的 [`PreparedLaunch`] 快照，供异常退出报告使用。
//!
//! 登录只依赖 [`dmcl_account`]：账户快照、登录源与离线 / 微软实现在那个 crate 里，
//! 本 crate 通过 [`LaunchChainOptions::account_source`] 取用。登录节点等文件补全与 natives
//! 都结束才开始，避免在线会话在长时间下载中过期。
//!
//! ```no_run
//! use std::sync::Arc;
//! use dmcl_account::OfflineAccountSource;
//! use dmcl_game::{GameFilesOptions, VersionRef};
//! use dmcl_java::JavaOption;
//! use dmcl_launch::{
//!     launch_chain, GameIntegrity, GameLaunchOptions, JvmCompatibility, LaunchChainOptions,
//!     MemoryOptions, ProcessOptions, ProcessPriority,
//! };
//!
//! # async fn run(root: std::path::PathBuf, game: Arc<GameFilesOptions>) -> anyhow::Result<()> {
//! let java = JavaOption {
//!     path: "/usr/bin/java".into(),
//!     major: 17,
//!     arch: dmcl_mcmeta::os::Arch::X86_64,
//! };
//! let version = VersionRef::new(&root, "1.20.1");
//! let game_dir = version.dir();
//! let chain = LaunchChainOptions {
//!     account_source: Arc::new(OfflineAccountSource::new("player")),
//!     version,
//!     java: Arc::new(java),
//!     jvm_compatibility: JvmCompatibility::default(),
//!     game_integrity: GameIntegrity::default(),
//!     memory: MemoryOptions::default(),
//! };
//! let process = ProcessOptions {
//!     working_dir: game_dir.clone(),
//!     priority: ProcessPriority::Normal,
//!     log_path: None,
//! };
//!
//! // 检查游戏完整性 → 登录 → 启动
//! let session = launch_chain(
//!     chain,
//!     GameLaunchOptions::default(),
//!     game_dir,
//!     process,
//!     game,
//!     None,
//! )
//! .run(|event| println!("{event}"))
//! .await?;
//! let mut session = std::sync::Arc::try_unwrap(session)
//!     .map_err(|_| anyhow::anyhow!("游戏会话句柄仍被共享"))?;
//! session.monitor.wait().await?;
//! # Ok(())
//! # }
//! ```

pub mod arguments;
pub mod chain;
pub mod files;
pub mod java;
pub mod memory;
pub mod monitor;

pub use arguments::{
    generate_arguments, GameLaunchOptions, GenerateArgsTask, PreparedLaunch, QuickPlay, WindowMode,
    LAUNCHER_NAME, LAUNCHER_VERSION,
};
pub use chain::{launch_chain, LaunchChainOptions, LoginTask, ReadVersionTask};
pub use files::{GameIntegrity, PrepareLaunchFilesTask, PrepareNativesTask};
pub use java::{JvmCompatibility, SelectJavaTask};
pub use memory::{
    auto_max_memory, resolve_memory, system_available_mib, MemoryOptions, MemorySize,
};
pub use monitor::{
    session_log_path, GameMonitor, GameSession, LaunchTask, ProcessOptions, ProcessPriority,
};
