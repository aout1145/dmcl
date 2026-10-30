//! 版本清单适配：把各数据源转成 `VersionItem`，以及取数器。

use std::sync::Arc;

use dmcl_mcmeta::source::DownloadSource;

use crate::gui::gtk4::pages::download::common::version_select::{
    Badge, FetchVersions, VersionItem,
};

/// Minecraft 版本清单的取数器：**忽略上下文**，一次取全清单。
pub(super) fn mc_fetcher(source: DownloadSource) -> FetchVersions {
    Arc::new(move |_context: &str| {
        let source = source.clone();
        Box::pin(async move {
            let manifest = source.fetch_version_manifest().await?;
            Ok(mc_items(manifest))
        })
    })
}

fn badge(key: &str, label: &str) -> Badge {
    Badge {
        key: key.to_string(),
        label: label.to_string(),
    }
}

/// 原版清单 → 条目：`latest.release` / `latest.snapshot` 置顶，类型合并「远古」。
pub(super) fn mc_items(
    manifest: dmcl_mcmeta::vanilla::manifest::VersionManifest,
) -> Vec<VersionItem> {
    let latest_release = manifest
        .latest
        .as_ref()
        .map(|latest| latest.release.clone())
        .unwrap_or_default();
    let latest_snapshot = manifest
        .latest
        .as_ref()
        .map(|latest| latest.snapshot.clone())
        .unwrap_or_default();

    manifest
        .versions
        .iter()
        .map(|entry| {
            // old_alpha / old_beta（以及部分镜像的 alpha / beta）统一并成「远古」，
            // 免得筛选 chips 跟着远程数据的措辞变化。
            let kind = match entry.kind.as_str() {
                "old_alpha" | "old_beta" | "alpha" | "beta" => "old",
                other => other,
            };
            let mut badges = Vec::new();
            if entry.id == latest_release {
                badges.push(badge("latest", "最新正式版"));
            } else if entry.id == latest_snapshot {
                badges.push(badge("latest", "最新预览版"));
            }
            VersionItem {
                id: entry.id.clone(),
                label: entry.id.clone(),
                kind: kind.to_string(),
                badges,
                detail: entry
                    .release_time
                    .as_deref()
                    .map(|time| time.chars().take(10).collect()),
            }
        })
        .collect()
}

/// Forge 列表 → 条目：第一条「最新版本」，`branch == "recommended"` 的「推荐版本」。
///
/// 展示与自动命名用 `version` 字段（如 `47.2.0`）；安装 id 仍是 `full_name`
/// （如 `1.20.1-47.2.0`）。
pub(super) fn forge_items(entries: Vec<dmcl_mcmeta::forge::ForgeVersionEntry>) -> Vec<VersionItem> {
    let first = entries.first().map(|entry| entry.full_name());
    let recommended = entries
        .iter()
        .find(|entry| entry.branch.as_deref() == Some("recommended"))
        .map(|entry| entry.full_name());

    entries
        .into_iter()
        .map(|entry| {
            let full_name = entry.full_name();
            let mut badges = Vec::new();
            if first.as_deref() == Some(full_name.as_str()) {
                badges.push(badge("latest", "最新版本"));
            }
            if recommended.as_deref() == Some(full_name.as_str()) {
                badges.push(badge("recommended", "推荐版本"));
            }
            VersionItem {
                id: full_name,
                label: entry.version,
                kind: String::new(),
                badges,
                detail: Some(format!("构建 {}", entry.build)),
            }
        })
        .collect()
}

/// Fabric 列表 → 条目：第一条「最新版本」，第一条 stable「推荐版本」；不区分类型。
pub(super) fn fabric_items(
    entries: Vec<dmcl_mcmeta::fabric::FabricLoaderEntry>,
) -> Vec<VersionItem> {
    let first_stable = entries
        .iter()
        .position(|entry| entry.loader.stable == Some(true));

    entries
        .into_iter()
        .enumerate()
        .map(|(index, entry)| {
            let mut badges = Vec::new();
            if index == 0 {
                badges.push(badge("latest", "最新版本"));
            }
            if first_stable == Some(index) {
                badges.push(badge("stable", "推荐版本"));
            }
            VersionItem {
                id: entry.loader.version.clone(),
                label: entry.loader.version,
                kind: String::new(),
                badges,
                detail: None,
            }
        })
        .collect()
}

/// NeoForge 列表 → 条目：第一条「最新版本」（已按版本号新→旧排序）。
pub(super) fn neoforge_items(
    entries: Vec<dmcl_mcmeta::neoforge::NeoForgeListEntry>,
) -> Vec<VersionItem> {
    entries
        .into_iter()
        .enumerate()
        .map(|(index, entry)| VersionItem {
            id: entry.version.clone(),
            label: entry.version,
            kind: String::new(),
            badges: if index == 0 {
                vec![badge("latest", "最新版本")]
            } else {
                Vec::new()
            },
            detail: None,
        })
        .collect()
}

/// OptiFine 列表 → 条目：最新稳定版、最新预览版各给一个置顶徽标。
pub(super) fn optifine_items(
    entries: Vec<dmcl_mcmeta::optifine::OptiFineVersionEntry>,
) -> Vec<VersionItem> {
    let latest_stable = entries
        .iter()
        .find(|entry| !entry.is_preview())
        .map(dmcl_mcmeta::optifine::OptiFineVersionEntry::version);
    let latest_preview = entries
        .iter()
        .find(|entry| entry.is_preview())
        .map(dmcl_mcmeta::optifine::OptiFineVersionEntry::version);

    entries
        .into_iter()
        .map(|entry| {
            let version = entry.version();
            let mut badges = Vec::new();
            if latest_stable.as_deref() == Some(version.as_str()) {
                badges.push(badge("latest-stable", "最新稳定版"));
            }
            if latest_preview.as_deref() == Some(version.as_str()) {
                badges.push(badge("latest-preview", "最新预览版"));
            }
            VersionItem {
                id: version.clone(),
                label: version,
                kind: if entry.is_preview() {
                    "preview".to_string()
                } else {
                    "stable".to_string()
                },
                badges,
                detail: entry.forge,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use dmcl_mcmeta::fabric::{FabricLoader, FabricLoaderEntry};
    use dmcl_mcmeta::forge::ForgeVersionEntry;
    use dmcl_mcmeta::neoforge::NeoForgeListEntry;
    use dmcl_mcmeta::optifine::OptiFineVersionEntry;
    use dmcl_mcmeta::vanilla::manifest::{LatestVersions, VersionManifest, VersionManifestEntry};

    fn labels(item: &VersionItem) -> Vec<&str> {
        item.badges
            .iter()
            .map(|badge| badge.label.as_str())
            .collect()
    }

    fn manifest_entry(id: &str, kind: &str) -> VersionManifestEntry {
        VersionManifestEntry {
            id: id.to_string(),
            kind: kind.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn mc_items_merge_old_kinds_and_badge_latest() {
        let manifest = VersionManifest {
            versions: vec![
                manifest_entry("24w00a", "snapshot"),
                manifest_entry("1.21.10", "release"),
                manifest_entry("a1.0", "old_alpha"),
            ],
            latest: Some(LatestVersions {
                release: "1.21.10".to_string(),
                snapshot: "24w00a".to_string(),
            }),
        };

        let items = mc_items(manifest);
        assert_eq!(labels(&items[0]), vec!["最新预览版"]);
        assert_eq!(labels(&items[1]), vec!["最新正式版"]);
        // old_alpha / old_beta 合并成 old，供「远古」chip 过滤。
        assert_eq!(items[2].kind, "old");
        assert!(items[2].badges.is_empty());
    }

    #[test]
    fn forge_items_badge_first_and_recommended() {
        let entries = vec![
            ForgeVersionEntry {
                mcversion: "1.20.1".to_string(),
                version: "47.2.0".to_string(),
                build: 3,
                ..Default::default()
            },
            ForgeVersionEntry {
                mcversion: "1.20.1".to_string(),
                version: "47.1.0".to_string(),
                branch: Some("recommended".to_string()),
                build: 2,
                ..Default::default()
            },
        ];

        let items = forge_items(entries);
        // 安装 id 仍是 full_name；展示与自动命名用 version 字段。
        assert_eq!(items[0].id, "1.20.1-47.2.0");
        assert_eq!(items[0].label, "47.2.0");
        assert_eq!(labels(&items[0]), vec!["最新版本"]);
        assert_eq!(labels(&items[1]), vec!["推荐版本"]);
        assert_eq!(items[1].id, "1.20.1-47.1.0-recommended");
        assert_eq!(items[1].label, "47.1.0");
        assert_eq!(items[1].detail.as_deref(), Some("构建 2"));
    }

    fn fabric_entry(version: &str, stable: bool) -> FabricLoaderEntry {
        FabricLoaderEntry {
            loader: FabricLoader {
                version: version.to_string(),
                stable: Some(stable),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn fabric_items_badge_first_and_first_stable() {
        let items = fabric_items(vec![
            fabric_entry("0.16.0", false),
            fabric_entry("0.15.11", true),
            fabric_entry("0.15.10", true),
        ]);

        // 第一条拿「最新版本」，第一条 stable 拿「推荐版本」；两者可以不是同一条。
        assert_eq!(labels(&items[0]), vec!["最新版本"]);
        assert_eq!(labels(&items[1]), vec!["推荐版本"]);
        assert!(items[2].badges.is_empty());
        // 加载器列表不区分版本类型，类型列留空。
        assert!(items.iter().all(|item| item.kind.is_empty()));
    }

    #[test]
    fn fabric_items_can_carry_both_badges_on_the_first_entry() {
        let items = fabric_items(vec![fabric_entry("0.16.0", true)]);
        assert_eq!(labels(&items[0]), vec!["最新版本", "推荐版本"]);
    }

    #[test]
    fn fabric_items_without_stable_only_badge_the_first() {
        let items = fabric_items(vec![
            fabric_entry("0.16.0", false),
            fabric_entry("0.15.0", false),
        ]);

        assert_eq!(labels(&items[0]), vec!["最新版本"]);
        assert!(items[1].badges.is_empty());
    }

    #[test]
    fn neoforge_items_badge_only_the_first() {
        let items = neoforge_items(vec![
            NeoForgeListEntry {
                version: "21.1.80".to_string(),
                ..Default::default()
            },
            NeoForgeListEntry {
                version: "21.1.79".to_string(),
                ..Default::default()
            },
        ]);

        assert_eq!(labels(&items[0]), vec!["最新版本"]);
        assert!(items[1].badges.is_empty());
    }

    fn optifine_entry(kind: &str, patch: &str, filename: &str) -> OptiFineVersionEntry {
        OptiFineVersionEntry {
            mcversion: "1.19".to_string(),
            kind: kind.to_string(),
            patch: patch.to_string(),
            filename: filename.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn optifine_items_badge_latest_stable_and_preview() {
        let items = optifine_items(vec![
            optifine_entry("HD_U", "H9", "OptiFine_1.19_HD_U_H9.jar"),
            optifine_entry("HD_U_H9", "pre2", "preview_OptiFine_1.19_HD_U_H9_pre2.jar"),
            optifine_entry("HD_U", "H8", "OptiFine_1.19_HD_U_H8.jar"),
        ]);

        assert_eq!(items[0].id, "HD_U_H9");
        assert_eq!(items[0].kind, "stable");
        assert_eq!(labels(&items[0]), vec!["最新稳定版"]);

        assert_eq!(items[1].id, "HD_U_H9_pre2");
        assert_eq!(items[1].kind, "preview");
        assert_eq!(labels(&items[1]), vec!["最新预览版"]);

        // 旧稳定版不再带徽标。
        assert!(items[2].badges.is_empty());
    }
}
