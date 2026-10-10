//! 版本清单（`version_manifest.json`）。只读。

use serde::Deserialize;

use crate::source::{templatize_urls, UrlFields};

/// Minecraft 的版本清单。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct VersionManifest {
    #[serde(default)]
    pub versions: Vec<VersionManifestEntry>,
    /// `latest` 块里的 release / snapshot（未建模的其它键忽略）。
    #[serde(default)]
    pub latest: Option<LatestVersions>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct LatestVersions {
    #[serde(default)]
    pub release: String,
    #[serde(default)]
    pub snapshot: String,
}

/// 清单里的一项。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct VersionManifestEntry {
    #[serde(default)]
    pub id: String,
    /// `release` / `snapshot` / `old_beta` … 取值由远程文档决定，因此保持字符串。
    #[serde(rename = "type", default)]
    pub kind: String,
    /// 该版本的版本 JSON 地址（已归一成 URL 模板）。
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub sha1: Option<String>,
    #[serde(default, rename = "releaseTime")]
    pub release_time: Option<String>,
}

impl UrlFields for VersionManifestEntry {
    fn for_each_url_mut(&mut self, f: &mut dyn FnMut(&mut String)) {
        f(&mut self.url);
    }
}

impl VersionManifest {
    /// 归一化所有版本 JSON 的地址。
    pub fn templatize_urls(&mut self) -> usize {
        let mut changed = 0;
        for entry in &mut self.versions {
            changed += templatize_urls(entry);
        }
        changed
    }

    /// 找出某个版本 —— 与原来的"从后往前找"一致（清单是按新到旧排的）。
    pub fn find(&self, version_id: &str) -> Option<&VersionManifestEntry> {
        self.versions.iter().rev().find(|ver| ver.id == version_id)
    }

    /// 按 `type` 过滤（`list-mc --type=release,snapshot`）。
    ///
    /// 保持"原样顺序"，由调用者决定是否倒序输出。
    pub fn filter_kinds<'a>(
        &'a self,
        kinds: &'a [String],
    ) -> impl Iterator<Item = &'a VersionManifestEntry> + 'a {
        self.versions
            .iter()
            .filter(move |ver| kinds.iter().any(|kind| kind == &ver.kind))
    }

    /// `latest.release` 指向的版本。
    pub fn latest_release(&self) -> Option<&VersionManifestEntry> {
        self.latest
            .as_ref()
            .map(|latest| latest.release.as_str())
            .filter(|id| !id.is_empty())
            .and_then(|id| self.find(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> VersionManifest {
        let mut manifest: VersionManifest = serde_json::from_str(
            r#"{
                "latest": {"release": "1.21.1", "snapshot": "24w00a"},
                "versions": [
                    {"id": "24w00a", "type": "snapshot",
                     "url": "https://piston-meta.mojang.com/x/24w00a.json"},
                    {"id": "1.21.1", "type": "release",
                     "url": "https://piston-meta.mojang.com/x/1.21.1.json"}
                ]
            }"#,
        )
        .unwrap();
        manifest.templatize_urls();
        manifest
    }

    #[test]
    fn urls_are_templatized_on_load() {
        let manifest = manifest();
        assert_eq!(manifest.versions[0].url, "${MC_META}/x/24w00a.json");
    }

    #[test]
    fn versions_can_be_looked_up_and_filtered() {
        let manifest = manifest();
        assert_eq!(manifest.find("1.21.1").unwrap().kind, "release");
        assert!(manifest.find("1.0").is_none());
        assert_eq!(manifest.latest_release().unwrap().id, "1.21.1");

        let kinds = ["release".to_string()];
        let releases: Vec<&str> = manifest
            .filter_kinds(&kinds)
            .map(|entry| entry.id.as_str())
            .collect();
        assert_eq!(releases, vec!["1.21.1"]);
    }
}
