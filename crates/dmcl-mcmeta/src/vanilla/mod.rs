//! 原版 Minecraft 的元数据：版本 JSON、版本清单、资源索引、规则。

pub mod asset;
pub mod manifest;
pub mod rules;
pub mod urls;
pub mod version;

pub use asset::{object_path, AssetIndexFile, AssetObject};
pub use manifest::{LatestVersions, VersionManifest, VersionManifestEntry};
pub use rules::{check_rules, rules_allow, OsRule, Rule, RuleAction, RuleContext, VersionRange};
pub use urls::{
    asset_index_url, asset_object_url, maven_library_url, VERSION_MANIFEST, VERSION_MANIFEST_V2,
};
pub use version::{
    Argument, Arguments, Artifact, AssetIndexRef, Extract, JavaVersion, Library, LibraryDownloads,
    Logging, LoggingClient, LoggingFile, RuledArgument, StringOrVec, VersionDownloads, VersionJson,
};
