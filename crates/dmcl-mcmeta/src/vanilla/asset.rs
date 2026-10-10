//! 资源索引（`assets/indexes/<id>.json`）。
//!
//! 真实文档里顶层只有 `objects` / `virtual` / `map_to_resources`，每个 object
//! 只有 `hash` / `size`；未建模的键仍会落进 `extra`，这样第三方或未来扩展的
//! 索引也不会在解析时丢字段。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::json::JsonMap;

/// `assets/indexes/<id>.json`。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AssetIndexFile {
    #[serde(default)]
    pub objects: BTreeMap<String, AssetObject>,
    /// `virtual` —— 为 true 时资源要铺到 `assets/virtual/<id>/`（`legacy` 索引）。
    #[serde(default, rename = "virtual", skip_serializing_if = "Option::is_none")]
    pub virtual_: Option<bool>,
    /// `map_to_resources` —— 为 true 时资源要铺到 `resources/`（`pre-1.6` 索引）。
    #[serde(
        default,
        rename = "map_to_resources",
        skip_serializing_if = "Option::is_none"
    )]
    pub map_to_resources: Option<bool>,
    #[serde(flatten)]
    pub extra: JsonMap,
}

impl AssetIndexFile {
    /// `json["virtual"].boolean == true`，也就是"存在且为 true"。
    pub fn is_virtual(&self) -> bool {
        self.virtual_.unwrap_or(false)
    }

    /// `json["map_to_resources"].boolean == true`。
    pub fn is_map_to_resources(&self) -> bool {
        self.map_to_resources.unwrap_or(false)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AssetObject {
    #[serde(default)]
    pub hash: String,
    #[serde(default)]
    pub size: u64,
    #[serde(flatten)]
    pub extra: JsonMap,
}

impl AssetObject {
    /// 资源对象在 `assets/objects` 下的相对路径：`<hash 前两位>/<hash>`。
    ///
    /// hash 太短（脏数据）时返回 `None`。
    pub fn path(&self) -> Option<String> {
        object_path(&self.hash)
    }
}

/// 由 hash 得到资源对象的相对路径。
pub fn object_path(hash: &str) -> Option<String> {
    if hash.len() < 2 {
        return None;
    }
    Some(format!("{}/{}", &hash[0..2], hash))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_index_reports_virtual_only_when_true() {
        let plain: AssetIndexFile =
            serde_json::from_str(r#"{"objects":{"a":{"hash":"aa","size":1}}}"#).unwrap();
        assert!(!plain.is_virtual());
        assert!(!plain.is_map_to_resources());
        let virtual_index: AssetIndexFile =
            serde_json::from_str(r#"{"virtual":true,"objects":{}}"#).unwrap();
        assert!(virtual_index.is_virtual());
        let explicit_false: AssetIndexFile =
            serde_json::from_str(r#"{"virtual":false,"objects":{}}"#).unwrap();
        assert!(!explicit_false.is_virtual());
        let legacy: AssetIndexFile =
            serde_json::from_str(r#"{"map_to_resources":true,"objects":{}}"#).unwrap();
        assert!(legacy.is_map_to_resources());
    }

    #[test]
    fn object_paths_follow_the_two_level_layout() {
        let object = AssetObject {
            hash: "abcdef0123".to_string(),
            size: 1,
            ..Default::default()
        };
        assert_eq!(object.path().as_deref(), Some("ab/abcdef0123"));
        assert_eq!(object_path("a"), None);
    }

    #[test]
    fn unknown_keys_round_trip() {
        // 真实的原版索引没有这些键，但第三方索引可能有；解析后写回不能丢。
        let index: AssetIndexFile = serde_json::from_str(
            r#"{"objects":{"a":{"hash":"aa","size":1,"note":"x"}},"futureFlag":true}"#,
        )
        .unwrap();
        assert_eq!(index.objects["a"].extra["note"], "x");
        assert_eq!(index.extra["futureFlag"], true);

        let written = serde_json::to_value(&index).unwrap();
        assert_eq!(written["objects"]["a"]["note"], "x");
        assert_eq!(written["futureFlag"], true);
        // 不存在的可选字段不会补出 null
        assert!(written.get("virtual").is_none());
        assert!(written.get("map_to_resources").is_none());
    }
}
