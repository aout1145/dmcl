//! OptiFine 的元数据、地址与取数（版本列表、下载地址）。
//!
//! OptiFine 的版本列表与下载只有 bmclapi 提供，因此这里不走 `${MC_*}` 模板，
//! 而是直接拼 bmclapi 的地址。

use anyhow::Result;
use serde::Deserialize;

use crate::json::JsonMap;
use crate::source::{DownloadSource, Mirror, Root};

/// bmclapi `/optifine/<mcversion>` 列表的一项。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct OptiFineVersionEntry {
    #[serde(default)]
    pub mcversion: String,
    /// API 的 `type` 字段：稳定版是 `HD_U`，预览版是 `HD_U_I6` 这种。
    #[serde(default, rename = "type")]
    pub kind: String,
    /// API 的 `patch` 字段：稳定版是 `I6`，预览版是 `pre4`。
    #[serde(default)]
    pub patch: String,
    /// 文件名，用来判断稳定版 / 预览版。
    #[serde(default)]
    pub filename: String,
    /// 官方推荐的 Forge 版本（可能为空）。
    #[serde(default)]
    pub forge: Option<String>,
    #[serde(flatten)]
    pub extra: JsonMap,
}

/// 主版本号（`H8` / `I6` / `J3` …），只用于给版本列表排序。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct MajorVersion {
    letter: char,
    number: u32,
}

impl OptiFineVersionEntry {
    /// 完整 OptiFine 版本：`<type>_<patch>`，例如 `HD_U_H8_pre2`。
    pub fn version(&self) -> String {
        if self.patch.is_empty() {
            self.kind.clone()
        } else {
            format!("{}_{}", self.kind, self.patch)
        }
    }

    /// 版本 JSON 里 `optifine:OptiFine:<...>` 的版本部分。
    pub fn library_version(&self, mc_version: &str) -> String {
        format!("{mc_version}_{}", self.version())
    }

    /// 是否预览版。
    pub fn is_preview(&self) -> bool {
        self.filename.starts_with("preview_") || self.patch.starts_with("pre")
    }

    /// bmclapi 上的下载地址。
    pub fn download_url(&self) -> String {
        optifine_download_url(&self.mcversion, &self.kind, &self.patch)
    }

    /// 从已经（或没有）排序的列表里挑最新版本：优先最新稳定版，没有稳定版才取最新预览版。
    ///
    /// `entries` 不需要预先排序。
    pub fn latest(entries: &[Self]) -> Option<&Self> {
        entries
            .iter()
            .filter(|entry| !entry.is_preview())
            .max_by_key(|entry| entry.sort_key())
            .or_else(|| entries.iter().max_by_key(|entry| entry.sort_key()))
    }

    /// 主版本；解析不出来时返回 `None`（排序时排在能解析的条目后面）。
    fn major(&self) -> Option<MajorVersion> {
        let version = self.version();
        let token = version
            .split('_')
            .rev()
            .find(|part| !part.is_empty() && !part.starts_with("pre"))?;
        let mut chars = token.chars();
        let letter = chars.next()?;
        let rest = chars.as_str();
        if letter.is_ascii_alphabetic()
            && !rest.is_empty()
            && rest.chars().all(|ch| ch.is_ascii_digit())
        {
            Some(MajorVersion {
                letter,
                number: rest.parse().ok()?,
            })
        } else {
            None
        }
    }

    /// 预览序号（`pre4` → 4）。
    fn preview_number(&self) -> Option<u32> {
        self.patch.strip_prefix("pre")?.parse().ok()
    }

    /// 从新到旧的排序键：主版本 > 稳定版优先于同主版本的预览版 > 预览序号。
    fn sort_key(&self) -> (bool, char, u32, bool, u32) {
        let (has_major, letter, number) = match self.major() {
            Some(major) => (true, major.letter, major.number),
            None => (false, '\0', 0),
        };
        (
            has_major,
            letter,
            number,
            !self.is_preview(),
            self.preview_number().unwrap_or(0),
        )
    }
}

/// OptiFine 版本列表：`https://bmclapi2.bangbang93.com/optifine/<mcversion>`。
pub fn optifine_version_list_url(mc_version: &str) -> String {
    format!(
        "{}/optifine/{mc_version}",
        Root::McMeta.base(Mirror::Bmclapi)
    )
}

/// OptiFine 下载地址：`https://bmclapi2.bangbang93.com/optifine/<mcversion>/<type>/<patch>`。
pub fn optifine_download_url(mc_version: &str, kind: &str, patch: &str) -> String {
    format!(
        "{}/optifine/{mc_version}/{kind}/{patch}",
        Root::McMeta.base(Mirror::Bmclapi)
    )
}

impl DownloadSource {
    /// 取 OptiFine 版本列表，按稳定版 / 预览版与版本号从新到旧排序并去重。
    pub async fn fetch_optifine_versions(
        &self,
        mc_version: &str,
    ) -> Result<Vec<OptiFineVersionEntry>> {
        let mut listing: Vec<OptiFineVersionEntry> = self
            .fetch_json(&optifine_version_list_url(mc_version))
            .await?;
        listing.sort_by_key(|entry| std::cmp::Reverse(entry.sort_key()));
        listing.dedup_by(|a, b| a.version() == b.version());
        Ok(listing)
    }

    /// 最新 OptiFine 版本：优先最新稳定版，没有稳定版才取最新预览版。
    pub async fn latest_optifine_version(&self, mc_version: &str) -> Result<String> {
        let listing = self.fetch_optifine_versions(mc_version).await?;
        OptiFineVersionEntry::latest(&listing)
            .map(OptiFineVersionEntry::version)
            .ok_or_else(|| anyhow::anyhow!("Minecraft {mc_version} 没有可用的 OptiFine 版本"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(kind: &str, patch: &str, filename: &str) -> OptiFineVersionEntry {
        OptiFineVersionEntry {
            mcversion: "1.19".to_string(),
            kind: kind.to_string(),
            patch: patch.to_string(),
            filename: filename.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn entries_expose_the_optifine_and_library_versions() {
        let stable = entry("HD_U", "H8", "OptiFine_1.19_HD_U_H8.jar");
        assert_eq!(stable.version(), "HD_U_H8");
        assert_eq!(stable.library_version("1.19"), "1.19_HD_U_H8");
        assert!(!stable.is_preview());

        let preview = entry("HD_U_H8", "pre2", "preview_OptiFine_1.19_HD_U_H8_pre2.jar");
        assert_eq!(preview.version(), "HD_U_H8_pre2");
        assert_eq!(preview.library_version("1.19"), "1.19_HD_U_H8_pre2");
        assert!(preview.is_preview());
    }

    #[test]
    fn urls_point_at_bmclapi_only() {
        let entry = entry("HD_U_H8", "pre2", "preview_OptiFine_1.19_HD_U_H8_pre2.jar");
        assert_eq!(
            optifine_version_list_url("1.19"),
            "https://bmclapi2.bangbang93.com/optifine/1.19"
        );
        assert_eq!(
            entry.download_url(),
            "https://bmclapi2.bangbang93.com/optifine/1.19/HD_U_H8/pre2"
        );
    }

    #[test]
    fn versions_sort_stable_first_then_previews() {
        let mut listing = vec![
            entry("HD_U", "H8", "OptiFine_1.19_HD_U_H8.jar"),
            entry("HD_U_H8", "pre2", "preview_OptiFine_1.19_HD_U_H8_pre2.jar"),
            entry("HD_U", "H9", "OptiFine_1.19_HD_U_H9.jar"),
            entry("HD_U_H9", "pre1", "preview_OptiFine_1.19_HD_U_H9_pre1.jar"),
            entry(
                "HD_U_H8",
                "pre10",
                "preview_OptiFine_1.19_HD_U_H8_pre10.jar",
            ),
        ];
        listing.sort_by_key(|entry| std::cmp::Reverse(entry.sort_key()));

        let versions: Vec<String> = listing.iter().map(OptiFineVersionEntry::version).collect();
        assert_eq!(
            versions,
            [
                "HD_U_H9",
                "HD_U_H9_pre1",
                "HD_U_H8",
                "HD_U_H8_pre10",
                "HD_U_H8_pre2",
            ]
        );
        assert_eq!(
            OptiFineVersionEntry::latest(&listing).map(OptiFineVersionEntry::version),
            Some("HD_U_H9".to_string())
        );
    }

    #[test]
    fn latest_falls_back_to_preview_without_a_stable_version() {
        let listing = vec![
            entry("HD_U_H8", "pre2", "preview_OptiFine_1.19_HD_U_H8_pre2.jar"),
            entry(
                "HD_U_H8",
                "pre10",
                "preview_OptiFine_1.19_HD_U_H8_pre10.jar",
            ),
        ];
        assert_eq!(
            OptiFineVersionEntry::latest(&listing).map(OptiFineVersionEntry::version),
            Some("HD_U_H8_pre10".to_string())
        );
    }

    #[test]
    fn unparseable_majors_still_have_a_deterministic_order() {
        let mut listing = [
            entry("HD_U", "H8", "OptiFine_1.19_HD_U_H8.jar"),
            entry("HD_U", "", "OptiFine_1.19_HD_U.jar"),
        ];
        listing.sort_by_key(|entry| std::cmp::Reverse(entry.sort_key()));
        assert_eq!(listing[0].version(), "HD_U_H8");
        assert_eq!(listing[1].version(), "HD_U");
    }
}
