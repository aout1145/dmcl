//! 原版 Minecraft 的元数据：版本 JSON、版本清单、资源索引、规则、Java 运行时。

pub mod asset;
pub mod jre;
pub mod manifest;
pub mod rules;
pub mod urls;
pub mod version;

pub use asset::{object_path, AssetIndexFile, AssetObject};
pub use jre::{
    current_jre_platform, jre_platform_key, JreAvailability, JreFileDownloads, JreFileType,
    JreManifest, JreManifestRef, JreRuntimeEntry, JreRuntimeFile, JreRuntimeManifest,
    JreRuntimeVersion, JRE_MANIFEST_URL,
};
pub use manifest::{LatestVersions, VersionManifest, VersionManifestEntry};
pub use rules::{check_rules, rules_allow, OsRule, Rule, RuleAction, RuleContext, VersionRange};
pub use urls::{asset_object_url, maven_library_url};
pub use version::{
    Argument, Arguments, Artifact, AssetIndexRef, Extract, JavaVersion, Library, LibraryDownloads,
    Logging, LoggingClient, LoggingFile, RuledArgument, StringOrVec, VersionDownloads, VersionJson,
};
