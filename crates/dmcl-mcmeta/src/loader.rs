//! 模组加载器的版本列表（只读）。
//!
//! 这些文档都只用来"列出可选版本 / 找最新版"，不写回任何东西，因此不保留未建模字段。

use serde::Deserialize;

/// `bmclapi .../forge/minecraft/<mcver>` 的一项。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ForgeVersionEntry {
    #[serde(default)]
    pub mcversion: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub branch: Option<String>,
    /// build 号 —— 不可能为负。
    #[serde(default)]
    pub build: u64,
}

impl ForgeVersionEntry {
    /// `bmclapiGetForgeVersionName` —— `<mcversion>-<version>[-<branch>]`。
    pub fn full_name(&self) -> String {
        let mut name = format!("{}-{}", self.mcversion, self.version);
        if let Some(branch) = self.branch.as_deref() {
            // `("branch" in ver.object && !ver["branch"].isNull())`
            name.push('-');
            name.push_str(branch);
        }
        name
    }
}

/// 官方 neoforged maven 的版本列表接口。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct NeoForgeVersionListing {
    #[serde(default)]
    pub versions: Vec<String>,
}

/// `bmclapi .../neoforge/list/<mcver>` 的一项。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct NeoForgeListEntry {
    #[serde(default)]
    pub version: String,
}

/// `meta.fabricmc.net/v2/versions/loader/<mcver>` 的一项。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct FabricLoaderEntry {
    #[serde(default)]
    pub loader: FabricLoader,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct FabricLoader {
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub stable: Option<bool>,
}

impl FabricLoader {
    pub fn is_stable(&self) -> bool {
        self.stable.unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forge_version_names_include_the_branch_when_present() {
        let entry: ForgeVersionEntry =
            serde_json::from_str(r#"{"mcversion":"1.7.10","version":"10.13.4.1614","build":1614}"#)
                .unwrap();
        assert_eq!(entry.full_name(), "1.7.10-10.13.4.1614");

        let entry: ForgeVersionEntry = serde_json::from_str(
            r#"{"mcversion":"1.20.1","version":"47.2.0","branch":"recommended","build":1637}"#,
        )
        .unwrap();
        assert_eq!(entry.full_name(), "1.20.1-47.2.0-recommended");

        // `"branch": null` 时不该多出一个横杠
        let entry: ForgeVersionEntry =
            serde_json::from_str(r#"{"mcversion":"1.20.1","version":"47.2.0","branch":null}"#)
                .unwrap();
        assert_eq!(entry.full_name(), "1.20.1-47.2.0");
    }

    #[test]
    fn fabric_loader_stability_defaults_to_false() {
        let list: Vec<FabricLoaderEntry> = serde_json::from_str(
            r#"[{"loader":{"version":"0.15.0","stable":true}},{"loader":{"version":"0.16.0"}}]"#,
        )
        .unwrap();
        assert!(list[0].loader.is_stable());
        assert!(!list[1].loader.is_stable());
    }
}
