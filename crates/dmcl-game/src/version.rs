//! 版本引用与已解析的版本描述。

use std::path::PathBuf;

use anyhow::Result;
use dmcl_mcmeta::layout::{read_version_json, version_dir, version_jar_path, version_json_path};
use dmcl_mcmeta::vanilla::VersionJson;

/// `.minecraft` 根目录 + 版本名。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionRef {
    pub root: PathBuf,
    /// 本地版本目录名（`versions/<name>`）。
    pub name: String,
}

impl VersionRef {
    pub fn new(root: impl Into<PathBuf>, name: impl Into<String>) -> Self {
        Self {
            root: root.into(),
            name: name.into(),
        }
    }

    pub fn dir(&self) -> PathBuf {
        version_dir(&self.root, &self.name)
    }

    pub fn json_path(&self) -> PathBuf {
        version_json_path(&self.root, self.name.as_str())
    }

    pub fn jar_path(&self) -> PathBuf {
        version_jar_path(&self.root, self.name.as_str())
    }
}

/// 从磁盘读出来、已处理 `inheritsFrom` 的版本。
///
/// 这是**版本描述**，不包含资源索引。资源索引是 `assetIndex` 指向的独立文档，
/// 可能不存在、未下载或已损坏，不能塞进「已解析版本」里当成一个 `Option`。
#[derive(Debug, Clone)]
pub struct ResolvedVersion {
    pub reference: VersionRef,
    pub json: VersionJson,
}

/// 读取版本 JSON、解析 `inheritsFrom`、归一化 URL。
///
/// 只读版本 JSON，不读资源索引，也不下载任何东西。
pub async fn resolve_version(reference: &VersionRef) -> Result<ResolvedVersion> {
    let mut json = read_version_json(&reference.json_path()).await?;

    // 保持现有行为：只解析一层 `inheritsFrom`。
    if let Some(orig) = json.inherits_from.clone() {
        let mut base = read_version_json(&version_json_path(&reference.root, &orig)).await?;
        base.merge(&json)?;
        json = base;
    }

    Ok(ResolvedVersion {
        reference: reference.clone(),
        json,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dmcl_mcmeta::layout::version_dir;

    fn work_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("dmcl-game-version-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    async fn write_json(root: &std::path::Path, name: &str, json: serde_json::Value) {
        let path = version_json_path(root, name);
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&path, serde_json::to_string(&json).unwrap())
            .await
            .unwrap();
    }

    #[test]
    fn a_version_reference_maps_to_the_official_layout() {
        let version = VersionRef::new("/mc", "1.20.1");
        assert_eq!(
            version.dir(),
            version_dir(std::path::Path::new("/mc"), "1.20.1")
        );
        assert_eq!(
            version.json_path(),
            PathBuf::from("/mc/versions/1.20.1/1.20.1.json")
        );
        assert_eq!(
            version.jar_path(),
            PathBuf::from("/mc/versions/1.20.1/1.20.1.jar")
        );
    }

    #[tokio::test]
    async fn a_version_without_inheritance_is_read_as_is() {
        let root = work_dir("plain");
        write_json(&root, "1.0", serde_json::json!({"id": "1.0"})).await;

        let resolved = resolve_version(&VersionRef::new(&root, "1.0"))
            .await
            .unwrap();
        assert_eq!(resolved.json.id.as_deref(), Some("1.0"));
        assert_eq!(resolved.reference.name, "1.0");

        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    #[tokio::test]
    async fn inherited_fields_come_from_the_parent_version() {
        let root = work_dir("inherit");
        write_json(
            &root,
            "base",
            serde_json::json!({
                "id": "base",
                "mainClass": "a.Main",
                "libraries": [{"name": "a:a:1"}]
            }),
        )
        .await;
        write_json(
            &root,
            "child",
            serde_json::json!({
                "id": "child",
                "inheritsFrom": "base",
                "mainClass": "b.Main"
            }),
        )
        .await;

        let resolved = resolve_version(&VersionRef::new(&root, "child"))
            .await
            .unwrap();
        assert_eq!(resolved.json.id.as_deref(), Some("child"));
        assert_eq!(resolved.json.main_class.as_deref(), Some("b.Main"));
        assert_eq!(resolved.json.libraries.len(), 1);

        let _ = tokio::fs::remove_dir_all(&root).await;
    }
}
