//! Minecraft 元数据 crate。
//!
//! 这里集中三件事，它们都属于"描述游戏是什么"的范畴，而不涉及"把文件写到磁盘上"：
//!
//! 1. **强类型文档** —— 版本 JSON、版本清单、资源索引、安装器档案、模组加载器版本列表。
//!    每个结构体都带 `#[serde(flatten)] extra`，因此回写时不会丢掉未建模的字段。
//! 2. **下载源与 URL 模板** —— [`DownloadSource`] 把"从哪儿下"从"下什么"里抽出来：
//!    文档里所有 url 都以 `${MC_MAVEN}` 这样的**模板**形式存在，由 [`Root`] +
//!    [`Mirror`] 决定最终解析成官方地址还是镜像地址。
//! 3. **本地目录布局** —— `versions/<name>/<name>.json` 之类的路径约定，以及读取版本 JSON
//!    （含 `inheritsFrom` 合并）。
//!
//! 这个 crate **不读配置文件**：`root_path`、镜像、并发度等一律由调用者通过参数传入。

pub(crate) mod macros;

pub mod asset;
pub mod install_profile;
pub mod layout;
pub mod loader;
pub mod manifest;
pub mod maven;
pub mod merge;
pub mod meta;
pub mod os;
pub mod rules;
pub mod source;
pub mod version;

pub use asset::{object_path, AssetIndexFile, AssetObject};
pub use install_profile::{DataEntry, DataPair, InstallEntry, InstallProfile, Processor, Side};
pub use layout::{
    absolute, assets_dir, assets_index_path, assets_objects_dir, assets_virtual_dir, libraries_dir,
    natives_dir, read_version_json, read_version_json_safe, version_dir, version_jar_path,
    version_json_path,
};
pub use loader::{
    FabricLoader, FabricLoaderEntry, ForgeVersionEntry, NeoForgeListEntry, NeoForgeVersionListing,
};
pub use manifest::{LatestVersions, VersionManifest, VersionManifestEntry};
pub use maven::{lib_name_get_name, lib_name_get_version, lib_name_to_path};
pub use merge::{merge_typed, merge_value};
pub use meta::{
    asset_index_url, asset_object_url, fabric_loader_list_url, fabric_profile_url,
    forge_installer_url, forge_version_list_url, maven_library_url, neoforge_installer_url,
    neoforge_version_list_url, neoforge_version_list_url_bmclapi,
    neoforge_version_list_url_official, FORGE_VERSION_LIST, NEOFORGE_VERSION_LIST_BMCLAPI,
    VERSION_MANIFEST, VERSION_MANIFEST_V2,
};
pub use os::{classpath_separator, get_os_name};
pub use rules::{check_rules, rules_allow, OsRule, Rule, RuleAction};
pub use source::{
    resolve_url, resolve_url_candidates, templatize_url, DownloadSource, Mirror, Root, UrlFields,
    UrlTemplate,
};
pub use version::{
    Argument, Arguments, Artifact, AssetIndexRef, JavaVersion, Library, LibraryDownloads,
    RuledArgument, StringOrVec, VersionDownloads, VersionJson,
};
