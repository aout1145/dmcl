//! Mojang 的 Java 运行时（JRE）文档。
//!
//! 这里有两层文档：
//!
//! 1. **产品索引**（`all.json`）—— 按平台列出各组件（`java-runtime-gamma` …）可用的
//!    运行时条目，每条指向一份运行时文件清单。见 [`JreManifest`]。
//! 2. **运行时文件清单**（`manifest.json`）—— 该运行时需要落盘的目录 / 文件 / 链接。
//!    见 [`JreRuntimeManifest`]。
//!
//! 产品索引的顶层键是 Mojang 自己的平台名（`linux` / `windows-x64` / `mac-os-arm64` …），
//! 这里统一翻译成 mcmeta 的 [`Os`] + [`Arch`]。翻译不出来的平台（如 `gamecore`）会被
//! 忽略 —— 桌面版用不到它们。

use std::collections::BTreeMap;

use anyhow::Result;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::json::JsonMap;
use crate::macros::string_enum;
use crate::os::{Arch, Os};
use crate::source::{templatize_urls, DownloadSource, UrlFields};
use crate::vanilla::version::Artifact;

/// Mojang 的 Java 运行时产品索引地址。
pub const JRE_MANIFEST_URL: &str =
    "${MC_META}/v1/products/java-runtime/2ec0cc96c44e5a76b9c8b7c39df7210883d12871/all.json";

/// Mojang 平台名 → mcmeta 平台。
///
/// `linux` 与 `mac-os` 在 Mojang 的命名里都表示 x86_64；32 位 / arm64 会显式带后缀。
fn platform_from_key(key: &str) -> Option<(Os, Arch)> {
    match key {
        "linux" => Some((Os::Linux, Arch::X86_64)),
        "linux-i386" => Some((Os::Linux, Arch::X86)),
        "mac-os" => Some((Os::MacOs, Arch::X86_64)),
        "mac-os-arm64" => Some((Os::MacOs, Arch::Aarch64)),
        "windows-x64" => Some((Os::Windows, Arch::X86_64)),
        "windows-x86" => Some((Os::Windows, Arch::X86)),
        "windows-arm64" => Some((Os::Windows, Arch::Aarch64)),
        _ => None,
    }
}

/// mcmeta 平台 → Mojang 平台键（`linux` / `windows-x64` / `mac-os-arm64` …）。
///
/// Mojang 没有提供全部组合（例如 Linux arm64），这些组合返回 `None`。
pub fn jre_platform_key(os: Os, arch: Arch) -> Option<&'static str> {
    match (os, arch) {
        (Os::Linux, Arch::X86_64) => Some("linux"),
        (Os::Linux, Arch::X86) => Some("linux-i386"),
        (Os::MacOs, Arch::X86_64) => Some("mac-os"),
        (Os::MacOs, Arch::Aarch64) => Some("mac-os-arm64"),
        (Os::Windows, Arch::X86_64) => Some("windows-x64"),
        (Os::Windows, Arch::X86) => Some("windows-x86"),
        (Os::Windows, Arch::Aarch64) => Some("windows-arm64"),
        _ => None,
    }
}

/// 当前平台的 JRE 平台键；Mojang 不提供该组合（如 Linux arm64）时返回 `None`。
pub fn current_jre_platform() -> Option<(Os, Arch)> {
    let os = Os::current();
    let arch = Arch::current()?;
    jre_platform_key(os, arch).map(|_| (os, arch))
}

/// 产品索引：`平台 → 组件 → 运行时条目`。
///
/// 只建模桌面版认识的平台；`gamecore` 之类的键在解析时丢弃。
#[derive(Debug, Clone, Default)]
pub struct JreManifest {
    /// 平台 → 组件名（`java-runtime-gamma` …）→ 运行时条目。
    pub platforms: BTreeMap<(Os, Arch), BTreeMap<String, Vec<JreRuntimeEntry>>>,
}

impl JreManifest {
    /// 某个组件在指定平台上的全部运行时条目。
    pub fn runtimes(&self, component: &str, platform: (Os, Arch)) -> Option<&[JreRuntimeEntry]> {
        self.platforms
            .get(&platform)?
            .get(component)
            .map(Vec::as_slice)
    }

    /// 某个组件在指定平台上的首选运行时条目。
    pub fn runtime(&self, component: &str, platform: (Os, Arch)) -> Option<&JreRuntimeEntry> {
        self.runtimes(component, platform)?.first()
    }

    /// 把所有运行时清单的地址归一成模板，返回改写的字段个数。
    pub fn templatize_urls(&mut self) -> usize {
        self.platforms
            .values_mut()
            .flat_map(|components| components.values_mut())
            .flatten()
            .map(templatize_urls)
            .sum()
    }
}

impl Serialize for JreManifest {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let platforms: BTreeMap<&str, &BTreeMap<String, Vec<JreRuntimeEntry>>> = self
            .platforms
            .iter()
            .filter_map(|(&(os, arch), components)| Some((jre_platform_key(os, arch)?, components)))
            .collect();
        platforms.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for JreManifest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let platforms: BTreeMap<String, BTreeMap<String, Vec<JreRuntimeEntry>>> =
            BTreeMap::deserialize(deserializer)?;
        Ok(Self {
            platforms: platforms
                .into_iter()
                .filter_map(|(key, components)| Some((platform_from_key(&key)?, components)))
                .collect(),
        })
    }
}

/// 产品索引里一个组件的运行时条目。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JreRuntimeEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub availability: Option<JreAvailability>,
    /// 该运行时文件清单的地址（加载后是 URL 模板）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest: Option<JreManifestRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<JreRuntimeVersion>,
    #[serde(flatten)]
    pub extra: JsonMap,
}

impl UrlFields for JreRuntimeEntry {
    fn for_each_url_mut(&mut self, f: &mut dyn FnMut(&mut String)) {
        if let Some(manifest) = &mut self.manifest {
            manifest.for_each_url_mut(f);
        }
    }
}

/// 运行时条目里的 `availability` 块（Mojang 的预取进度信息）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JreAvailability {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<u32>,
    #[serde(flatten)]
    pub extra: JsonMap,
}

/// 运行时文件清单的下载信息：地址 + 校验值 + 大小。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JreManifestRef {
    /// 加载后是 URL 模板。
    #[serde(default)]
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha1: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(flatten)]
    pub extra: JsonMap,
}

impl UrlFields for JreManifestRef {
    fn for_each_url_mut(&mut self, f: &mut dyn FnMut(&mut String)) {
        f(&mut self.url);
    }
}

/// 运行时条目里的 `version` 块。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JreRuntimeVersion {
    /// JRE 自己的版本号，例如 `17.0.15`。
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub released: Option<String>,
    #[serde(flatten)]
    pub extra: JsonMap,
}

/// 运行时文件清单（`manifest.json`）：`相对路径 → 条目`。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JreRuntimeManifest {
    #[serde(default)]
    pub files: BTreeMap<String, JreRuntimeFile>,
    #[serde(flatten)]
    pub extra: JsonMap,
}

impl JreRuntimeManifest {
    /// 把所有下载地址归一成模板，返回改写的字段个数。
    pub fn templatize_urls(&mut self) -> usize {
        self.files.values_mut().map(templatize_urls).sum()
    }
}

/// 文件清单里的一项：目录、文件或符号链接。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JreRuntimeFile {
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<JreFileType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub downloads: Option<JreFileDownloads>,
    /// 文件是否要带上可执行位。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executable: Option<bool>,
    /// 链接目标（`type = link`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(flatten)]
    pub extra: JsonMap,
}

impl UrlFields for JreRuntimeFile {
    fn for_each_url_mut(&mut self, f: &mut dyn FnMut(&mut String)) {
        if let Some(downloads) = &mut self.downloads {
            downloads.for_each_url_mut(f);
        }
    }
}

string_enum! {
    /// 文件清单里的 `type`。
    JreFileType {
        Directory => "directory",
        File => "file",
        Link => "link",
    }
}

/// 文件条目的 `downloads`：同一份内容的原始版与 lzma 压缩版。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JreFileDownloads {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<Artifact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lzma: Option<Artifact>,
    #[serde(flatten)]
    pub extra: JsonMap,
}

impl UrlFields for JreFileDownloads {
    fn for_each_url_mut(&mut self, f: &mut dyn FnMut(&mut String)) {
        if let Some(raw) = &mut self.raw {
            raw.for_each_url_mut(f);
        }
        if let Some(lzma) = &mut self.lzma {
            lzma.for_each_url_mut(f);
        }
    }
}

impl DownloadSource {
    /// 取 Java 运行时产品索引；返回的文档里所有清单地址都已经是模板。
    pub async fn fetch_jre_manifest(&self) -> Result<JreManifest> {
        let mut manifest: JreManifest = self.fetch_json(JRE_MANIFEST_URL).await?;
        manifest.templatize_urls();
        Ok(manifest)
    }

    /// 取某个运行时的文件清单（`url` 是产品索引里的模板地址）。
    pub async fn fetch_jre_runtime_manifest(&self, url: &str) -> Result<JreRuntimeManifest> {
        let mut manifest: JreRuntimeManifest = self.fetch_json(url).await?;
        manifest.templatize_urls();
        Ok(manifest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_keys_translate_to_os_and_arch() {
        let mut manifest: JreManifest = serde_json::from_str(
            r#"{
                "linux": {"java-runtime-gamma": [{
                    "manifest": {
                        "url": "https://piston-meta.mojang.com/v1/packages/abc/manifest.json",
                        "sha1": "abc",
                        "size": 10
                    },
                    "version": {"name": "17.0.15", "released": "2025-05-19T08:24:16+00:00"}
                }]},
                "mac-os-arm64": {"java-runtime-gamma": []},
                "gamecore": {"java-runtime-gamma": []}
            }"#,
        )
        .unwrap();

        assert_eq!(manifest.platforms.len(), 2);
        let entry = manifest
            .runtime("java-runtime-gamma", (Os::Linux, Arch::X86_64))
            .unwrap();
        assert_eq!(entry.version.as_ref().unwrap().name, "17.0.15");
        assert!(manifest
            .runtimes("java-runtime-gamma", (Os::MacOs, Arch::Aarch64))
            .unwrap()
            .is_empty());
        // Mojang 没有 Linux arm64 的运行时；翻译不出来的平台键也被丢掉
        assert!(manifest
            .runtime("java-runtime-gamma", (Os::Linux, Arch::Aarch64))
            .is_none());

        manifest.templatize_urls();
        assert_eq!(
            manifest
                .runtime("java-runtime-gamma", (Os::Linux, Arch::X86_64))
                .unwrap()
                .manifest
                .as_ref()
                .unwrap()
                .url,
            "${MC_META}/v1/packages/abc/manifest.json"
        );

        // 回写时恢复成 Mojang 的平台键，未建模的键不再写出
        let written = serde_json::to_value(&manifest).unwrap();
        assert!(written.get("linux").is_some());
        assert!(written.get("mac-os-arm64").is_some());
        assert!(written.get("gamecore").is_none());
    }

    #[test]
    fn runtime_files_cover_directories_files_and_links() {
        let mut manifest: JreRuntimeManifest = serde_json::from_str(
            r#"{
                "files": {
                    "bin": {"type": "directory"},
                    "bin/java": {
                        "type": "file",
                        "executable": true,
                        "downloads": {
                            "raw": {
                                "sha1": "raw",
                                "size": 10,
                                "url": "https://piston-data.mojang.com/v1/objects/raw/java"
                            },
                            "lzma": {
                                "sha1": "lzma",
                                "size": 5,
                                "url": "https://piston-data.mojang.com/v1/objects/lzma/java"
                            }
                        }
                    },
                    "legal/LICENSE": {"type": "link", "target": "../java.base/LICENSE"},
                    "unknown": {"type": "socket"}
                }
            }"#,
        )
        .unwrap();

        assert_eq!(manifest.files["bin"].kind, Some(JreFileType::Directory));
        let java = &manifest.files["bin/java"];
        assert_eq!(java.kind, Some(JreFileType::File));
        assert_eq!(java.executable, Some(true));
        assert_eq!(
            manifest.files["legal/LICENSE"].target.as_deref(),
            Some("../java.base/LICENSE")
        );
        assert_eq!(
            manifest.files["unknown"].kind,
            Some(JreFileType::Other("socket".to_string()))
        );

        manifest.templatize_urls();
        let downloads = manifest.files["bin/java"].downloads.as_ref().unwrap();
        assert_eq!(
            downloads.raw.as_ref().unwrap().url.as_deref(),
            Some("${MC_DATA}/v1/objects/raw/java")
        );
        assert_eq!(
            downloads.lzma.as_ref().unwrap().url.as_deref(),
            Some("${MC_DATA}/v1/objects/lzma/java")
        );
    }
}
