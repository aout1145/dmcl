//! 资源索引（`assets/indexes/<id>.json`）。只读，不会被写回，因此不需要保留未建模字段。

use std::collections::BTreeMap;

use serde::Deserialize;

/// `assets/indexes/<id>.json`。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AssetIndexFile {
    #[serde(default)]
    pub objects: BTreeMap<String, AssetObject>,
    #[serde(default, rename = "virtual")]
    pub virtual_: Option<bool>,
    #[serde(default, rename = "map_to_resources")]
    pub map_to_resources: Option<bool>,
}

impl AssetIndexFile {
    /// `json["virtual"].boolean == true`，也就是"存在且为 true"。
    pub fn is_virtual(&self) -> bool {
        self.virtual_.unwrap_or(false)
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct AssetObject {
    #[serde(default)]
    pub hash: String,
    #[serde(default)]
    pub size: u64,
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
        let virtual_index: AssetIndexFile =
            serde_json::from_str(r#"{"virtual":true,"objects":{}}"#).unwrap();
        assert!(virtual_index.is_virtual());
        let explicit_false: AssetIndexFile =
            serde_json::from_str(r#"{"virtual":false,"objects":{}}"#).unwrap();
        assert!(!explicit_false.is_virtual());
    }

    #[test]
    fn object_paths_follow_the_two_level_layout() {
        let object = AssetObject {
            hash: "abcdef0123".to_string(),
            size: 1,
        };
        assert_eq!(object.path().as_deref(), Some("ab/abcdef0123"));
        assert_eq!(object_path("a"), None);
    }
}
