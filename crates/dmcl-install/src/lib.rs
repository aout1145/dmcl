//! 在磁盘上装出一个可玩的 Minecraft 版本：原版、Fabric、Forge、NeoForge。
//!
//! 这个 crate 只做"把文件写下去"，不做任何决策：
//!
//! * 装什么、从哪儿下 → 来自 [`dmcl_mcmeta`]（元数据 + [`dmcl_mcmeta::source::DownloadSource`]）；
//! * 怎么并发下 → 来自 [`dmcl_download`]；
//! * 进度怎么报、任务怎么串 → 来自 [`dmcl_task`]；
//! * 配置（`.`minecraft` 路径、镜像、并发度、是否跳过音乐）→ 由调用者通过
//!   [`InstallOptions`] 传入，**这个 crate 不读任何配置文件**。
//!
//! # 两种用法
//!
//! 直接拿现成的任务链（cli 走这条）：
//!
//! ```no_run
//! use std::sync::Arc;
//! use dmcl_install::{install_task, InstallOptions, InstallSpec, LoaderKind};
//! use dmcl_mcmeta::source::{DownloadSource, Mirror};
//!
//! # async fn run() -> anyhow::Result<()> {
//! let options = Arc::new(InstallOptions::new("/tmp/.minecraft", DownloadSource::new(Mirror::Bmclapi)));
//! let spec = InstallSpec::new("1.20.1", "1.20.1")
//!     .with_loader(LoaderKind::Fabric, None);
//!
//! install_task(options, spec)
//!     .run_with((), |event| println!("{event}"))
//!     .await
//! # }
//! ```
//!
//! 或者自己拼链条（gui 想自定义步骤时）：
//!
//! ```no_run
//! use std::sync::Arc;
//! use dmcl_install::{files::game_files_task, vanilla::vanilla_metadata_task, InstallOptions};
//! use dmcl_mcmeta::source::{DownloadSource, Mirror};
//! use dmcl_task::TaskChain;
//!
//! # async fn run() -> anyhow::Result<()> {
//! let options = Arc::new(InstallOptions::new("/tmp/.minecraft", DownloadSource::new(Mirror::Official)));
//! let chain = TaskChain::new("安装")
//!     .then(vanilla_metadata_task(options.clone(), "1.20.1".into(), "1.20.1".into()), "获取版本元数据")
//!     .then(game_files_task(options.clone(), "1.20.1".into()), "下载游戏文件");
//! chain.run(()).await
//! # }
//! ```
//!
//! # 设计要点
//!
//! * 把「这个版本需要哪些文件」的**规划**（[`plan`]，纯函数、可单测）与**下载**
//!   （[`files`]，构造 [`dmcl_download::DownloadTask`]）分开；
//! * natives 的解压归这里做 —— 它本质上就是「把文件铺好」，这样 `dmcl-launch`
//!   就不需要 zip 代码；
//! * Forge / NeoForge 的 processor 用 [`dmcl_task::CommandTask`] 跑，
//!   因此安装器的输出会作为日志事件流出来。

pub mod files;
pub mod integrity;
pub mod io;
mod loader;
pub mod options;
pub mod plan;
pub mod spec;
pub mod vanilla;
pub mod zip;

pub use files::{
    download_game_files, download_planned, ensure_asset_index, extract_natives, game_files_task,
};
pub use integrity::{check_integrity, IntegrityStatus, MissingFile, MissingReason};
pub use io::{write_json_safe, write_safe};
pub use loader::{fabric, forge, neoforge};
pub use options::{FixedJava, InstallOptions, JavaProvider, PathJava};
pub use plan::{
    native_artifact, plan_asset_objects, plan_assets_index, plan_client_jar, plan_game_files,
    plan_libraries, plan_natives, PlanContext, PlannedFile,
};
pub use spec::{install_loader_task, install_task, InstallSpec, LoaderKind, LoaderRequest};
pub use vanilla::{download_vanilla, generate_launcher_profiles, LAUNCHER_PROFILES_JSON};
