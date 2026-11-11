//! 一个已安装 Minecraft 版本的领域对象与文件物化。
//!
//! 这个 crate 回答：
//!
//! * 一个已安装版本在磁盘上是什么（[`VersionRef`] / [`ResolvedVersion`]）；
//! * 这个版本应该有哪些文件（[`plan_files`]，纯函数）；
//! * 本地缺了什么 / 坏了什么（[`check_integrity`]）；
//! * 怎么把缺的文件补齐（[`ensure_files`] / [`ensure_client_jar`] / [`ensure_libraries`] /
//!   [`ensure_assets`] / [`ensure_asset_index`]）；
//! * 它对应的 Minecraft 版本是什么、装了哪些加载器（[`detect_version`]）；
//! * natives 怎么按 os / arch 下载并解压（[`ensure_natives`]）。
//!
//! 它不创建版本目录、不写原版元数据、不安装加载器或 JRE，也不认识账户与启动参数。
//! 配置（下载源、并发策略、是否跳过音乐）由调用者通过 [`GameFilesOptions`] 传入，
//! **这个 crate 不读任何配置文件**。

mod class_parse;
mod detect;
pub mod ensure;
mod extract;
pub mod integrity;
pub mod natives;
pub mod options;
pub mod plan;
pub mod version;

pub use detect::{detect_version, DetectedLoader, DetectedLoaderKind, DetectedVersion};
pub use ensure::{
    ensure_asset_index, ensure_assets, ensure_client_jar, ensure_files, ensure_libraries,
};
pub use integrity::{check_integrity, IntegrityStatus, MissingFile, MissingReason};
pub use natives::ensure_natives;
pub use options::{download_planned, GameFilesOptions};
pub use plan::{
    base_name, native_artifact, plan_asset_objects, plan_assets_index, plan_client_jar, plan_files,
    plan_libraries, plan_native_jars, plan_natives, PlannedFile,
};
pub use version::{main_jar_path, main_jar_path_for, resolve_version, ResolvedVersion, VersionRef};
