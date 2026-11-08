//! 版本引用与已解析的版本描述。

use std::path::PathBuf;

use anyhow::Result;
use dmcl_mcmeta::layout::{
    read_resolved_version_json, version_dir, version_jar_path, version_json_path,
};
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
}

/// 客户端主 jar 的位置。
///
/// `VersionJson.jar` 表示由哪个版本提供客户端类；没有值（或空串）时用当前版本名。
/// 不读取 `inherits_from`。
pub fn main_jar_path(reference: &VersionRef, json: &VersionJson) -> PathBuf {
    main_jar_path_for(reference, json.jar.as_deref())
}

/// 只要知道 `jar` 字段时的主 jar 路径；库规划等不需要整份版本 JSON 的场景用它。
pub fn main_jar_path_for(reference: &VersionRef, jar: Option<&str>) -> PathBuf {
    let name = jar
        .filter(|name| !name.is_empty())
        .unwrap_or(reference.name.as_str());
    version_jar_path(&reference.root, name)
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

/// 读取版本 JSON、递归解析整条 `inheritsFrom` 链、归一化 URL。
///
/// 只读版本 JSON，不读资源索引，也不下载任何东西。
pub async fn resolve_version(reference: &VersionRef) -> Result<ResolvedVersion> {
    let json = read_resolved_version_json(&reference.root, &reference.name).await?;
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

    #[test]
    fn the_main_jar_prefers_json_jar_with_a_version_name_fallback() {
        let reference = VersionRef::new("/mc", "1.0");
        let mut json = VersionJson::default();

        assert_eq!(
            main_jar_path(&reference, &json),
            PathBuf::from("/mc/versions/1.0/1.0.jar")
        );

        json.jar = Some("base".to_string());
        assert_eq!(
            main_jar_path(&reference, &json),
            PathBuf::from("/mc/versions/base/base.jar")
        );

        json.jar = Some(String::new());
        assert_eq!(
            main_jar_path(&reference, &json),
            PathBuf::from("/mc/versions/1.0/1.0.jar")
        );
    }

    #[tokio::test]
    async fn the_resolved_main_jar_follows_the_root_ancestor() {
        let root = work_dir("root-jar");
        write_json(
            &root,
            "vanilla",
            serde_json::json!({
                "id": "vanilla",
                "libraries": [{"name": "v:lib:1"}]
            }),
        )
        .await;
        write_json(
            &root,
            "forge",
            serde_json::json!({
                "id": "forge",
                "inheritsFrom": "vanilla",
                "libraries": [{"name": "f:lib:1"}]
            }),
        )
        .await;
        write_json(
            &root,
            "optifine",
            serde_json::json!({
                "id": "optifine",
                "inheritsFrom": "forge",
                "libraries": [{"name": "o:lib:1"}]
            }),
        )
        .await;

        let resolved = resolve_version(&VersionRef::new(&root, "optifine"))
            .await
            .unwrap();

        assert_eq!(resolved.json.jar.as_deref(), Some("vanilla"));
        assert_eq!(
            main_jar_path(&resolved.reference, &resolved.json),
            root.join("versions").join("vanilla").join("vanilla.jar")
        );

        let _ = tokio::fs::remove_dir_all(&root).await;
    }
}
