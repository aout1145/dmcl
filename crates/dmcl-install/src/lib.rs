//! 在磁盘上装出一个可玩的 Minecraft 版本：原版、Fabric、Forge、NeoForge、OptiFine。
//!
//! 这个 crate 只做「把版本装起来」，不做任何决策：
//!
//! * 装什么、从哪儿下 → 来自 [`dmcl_mcmeta`]（元数据 + [`dmcl_mcmeta::source::DownloadSource`]）；
//! * 怎么并发下 → 来自 [`dmcl_download`]；
//! * 这个版本应该有哪些文件、怎么补齐 → 来自 [`dmcl_game`]；
//! * 安装器 processor 用哪个 Java、缺 Java 时怎么装 → 来自 [`dmcl_java`]；
//! * 进度怎么报、任务怎么串 → 来自 [`dmcl_task`]；
//! * 配置（`.`minecraft` 路径、镜像、并发度、是否跳过音乐）→ 由调用者通过
//!   [`InstallOptions`] 传入，**这个 crate 不读任何配置文件**。
//!
//! # 用法
//!
//! 直接拿现成的任务链（cli 走这条）：
//!
//! ```no_run
//! use std::sync::Arc;
//! use dmcl_game::GameFilesOptions;
//! use dmcl_install::{install_task, InstallOptions, InstallSpec, LoaderKind};
//! use dmcl_java::{JavaOption, JreAutoInstall};
//! use dmcl_mcmeta::os::Arch;
//! use dmcl_mcmeta::source::{DownloadSource, Mirror};
//!
//! # async fn run() -> anyhow::Result<()> {
//! let game = GameFilesOptions::new(DownloadSource::new(Mirror::Bmclapi));
//! let java = Arc::new(JavaOption {
//!     path: "/usr/bin/java".into(),
//!     major: 17,
//!     arch: Arch::X86_64,
//! });
//! let options = Arc::new(InstallOptions::new(
//!     "/tmp/.minecraft",
//!     game,
//!     java,
//!     JreAutoInstall::new("/tmp/.minecraft/runtime"),
//! ));
//! let spec = InstallSpec::new("1.20.1", "1.20.1")
//!     .with_loader(LoaderKind::Fabric, "0.15.0");
//!
//! install_task(options, spec)
//!     .run_with((), |event| println!("{event}"))
//!     .await
//!     .map(|_| ())
//! # }
//! ```
//!
//! 或者自己拼链条（gui 想自定义步骤时）：
//!
//! ```no_run
//! use dmcl_game::{ensure_files, resolve_version, VersionRef};
//! use dmcl_mcmeta::source::{DownloadSource, Mirror};
//! use dmcl_install::InstallOptions;
//! use dmcl_java::{JavaOption, JreAutoInstall};
//! use dmcl_mcmeta::os::Arch;
//! use dmcl_task::TaskContext;
//! use std::sync::Arc;
//!
//! # async fn run() -> anyhow::Result<()> {
//! let java = Arc::new(JavaOption {
//!     path: "/usr/bin/java".into(),
//!     major: 17,
//!     arch: Arch::X86_64,
//! });
//! let options = InstallOptions::new(
//!     "/tmp/.minecraft",
//!     dmcl_game::GameFilesOptions::new(DownloadSource::new(Mirror::Official)),
//!     java,
//!     JreAutoInstall::new("/tmp/.minecraft/runtime"),
//! );
//! let version = resolve_version(&VersionRef::new("/tmp/.minecraft", "1.20.1")).await?;
//! ensure_files(&version, &options.game, &TaskContext::detached("游戏文件")).await?;
//! # Ok(())
//! # }
//! ```
//!
//! # 设计要点
//!
//! * 「这个版本由哪些文件组成」与「缺了什么」由 [`dmcl_game`] 负责，安装器只负责
//!   原版元数据、加载器与版本目录的原子性；
//! * [`install_task`] 是**版本级原子**的：版本目录已存在就直接报错；只有占用成功后才走到
//!   `on_rollback` 登记中止回滚，失败或被取消时删掉本次新建的 `versions/<名称>`，共享的
//!   `libraries/` / `assets/` 作为校验缓存保留；
//! * Forge / NeoForge 的 processor 用 [`dmcl_task::CommandTask`] 跑，
//!   因此安装器的输出会作为日志事件流出来。

mod archive;
pub mod io;
mod loader;
pub mod options;
pub mod spec;
pub mod vanilla;

pub use io::{write_json_safe, write_safe};
pub use loader::{fabric, forge, neoforge, optifine};
pub use options::InstallOptions;
pub use spec::{
    default_version_name, install_task, InstallOutcome, InstallSpec, LoaderKind, LoaderRequest,
};
pub use vanilla::{download_vanilla, generate_launcher_profiles, LAUNCHER_PROFILES_JSON};
