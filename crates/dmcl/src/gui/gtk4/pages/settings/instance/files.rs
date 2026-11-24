//! 实例目录的重命名与删除。
//!
//! 重命名会同时改版本目录、`<名称>.json` / `<名称>.jar` 的文件名，以及版本 JSON 里等于
//! 旧名称的 `id` / `jar`。`dmcl.json`、`logo.png` 等其它文件跟着目录一起移动；其它版本
//! 写在 `inheritsFrom` 里的引用不会跟着改。

use std::path::Path;

use anyhow::{bail, Context, Result};
use dmcl_install::{validate_version_name, write_json_safe};
use dmcl_mcmeta::layout::version_dir;
use serde_json::Value;

/// 把实例 `versions/<old>` 重命名为 `versions/<new>`。
pub(crate) async fn rename_instance(root: &Path, old: &str, new: &str) -> Result<()> {
    if old == new {
        return Ok(());
    }
    validate_version_name(new)?;

    let old_dir = version_dir(root, old);
    let new_dir = version_dir(root, new);
    if tokio::fs::metadata(&new_dir).await.is_ok() {
        bail!("实例「{new}」已存在");
    }

    let old_json = old_dir.join(format!("{old}.json"));
    let text = tokio::fs::read_to_string(&old_json)
        .await
        .with_context(|| format!("无法读取版本 JSON {}", old_json.display()))?;
    let mut value: Value = serde_json::from_str(&text)
        .with_context(|| format!("无法解析版本 JSON {}", old_json.display()))?;
    let Some(document) = value.as_object_mut() else {
        bail!("版本 JSON 的顶层不是对象：{}", old_json.display());
    };
    if document.get("id").and_then(Value::as_str) == Some(old) {
        document.insert("id".to_string(), Value::String(new.to_string()));
    }
    if document.get("jar").and_then(Value::as_str) == Some(old) {
        document.insert("jar".to_string(), Value::String(new.to_string()));
    }

    // 先在新文件名下写好内容，再改目录名：目录一换过去，`<new>/<new>.json` 就已经可用。
    let staged_json = old_dir.join(format!("{new}.json"));
    write_json_safe(&staged_json, &value).await?;

    let old_jar = old_dir.join(format!("{old}.jar"));
    let new_jar = old_dir.join(format!("{new}.jar"));
    let jar_renamed = match tokio::fs::rename(&old_jar, &new_jar).await {
        Ok(()) => true,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => false,
        Err(err) => {
            let _ = tokio::fs::remove_file(&staged_json).await;
            return Err(err).with_context(|| format!("无法重命名版本 jar {}", old_jar.display()));
        }
    };

    if let Err(err) = tokio::fs::rename(&old_dir, &new_dir).await {
        if jar_renamed {
            let _ = tokio::fs::rename(&new_jar, &old_jar).await;
        }
        let _ = tokio::fs::remove_file(&staged_json).await;
        return Err(err).with_context(|| format!("无法重命名版本目录 {}", old_dir.display()));
    }

    // 旧 JSON 是多余的；删不掉也不影响新实例，留给用户手动清理。
    let _ = tokio::fs::remove_file(new_dir.join(format!("{old}.json"))).await;
    Ok(())
}

/// 删除整个实例目录；目录已经不存在时也视为成功。
pub(crate) async fn delete_instance(root: &Path, name: &str) -> Result<()> {
    let dir = version_dir(root, name);
    match tokio::fs::remove_dir_all(&dir).await {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err).with_context(|| format!("无法删除实例目录 {}", dir.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dmcl_mcmeta::layout::version_dir;
    use serde_json::json;

    fn root(tag: &str) -> std::path::PathBuf {
        let root =
            std::env::temp_dir().join(format!("dmcl-instance-files-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(version_dir(&root, "old")).unwrap();
        root
    }

    fn write_instance(root: &Path, name: &str, json: Value) {
        std::fs::write(
            version_dir(root, name).join(format!("{name}.json")),
            serde_json::to_string(&json).unwrap(),
        )
        .unwrap();
    }

    #[tokio::test]
    async fn rename_moves_the_directory_and_rewrites_id_and_jar() {
        let root = root("rename");
        let dir = version_dir(&root, "old");
        write_instance(
            &root,
            "old",
            json!({"id": "old", "jar": "old", "mainClass": "A"}),
        );
        std::fs::write(dir.join("old.jar"), b"jar").unwrap();
        std::fs::write(dir.join("dmcl.json"), r#"{"launch":{}}"#).unwrap();
        std::fs::write(dir.join("logo.png"), b"png").unwrap();

        rename_instance(&root, "old", "new").await.unwrap();

        assert!(!dir.exists());
        let new_dir = version_dir(&root, "new");
        assert!(new_dir.join("dmcl.json").is_file());
        assert!(new_dir.join("logo.png").is_file());
        assert!(new_dir.join("new.jar").is_file());
        assert!(!new_dir.join("old.json").exists());

        let document: Value =
            serde_json::from_str(&std::fs::read_to_string(new_dir.join("new.json")).unwrap())
                .unwrap();
        assert_eq!(document["id"], "new");
        assert_eq!(document["jar"], "new");
        assert_eq!(document["mainClass"], "A");

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[tokio::test]
    async fn rename_keeps_ids_that_point_elsewhere() {
        let root = root("foreign");
        write_instance(&root, "old", json!({"id": "parent", "jar": "parent"}));

        rename_instance(&root, "old", "new").await.unwrap();

        let document: Value = serde_json::from_str(
            &std::fs::read_to_string(version_dir(&root, "new").join("new.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(document["id"], "parent");
        assert_eq!(document["jar"], "parent");

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[tokio::test]
    async fn rename_rejects_invalid_names_and_existing_targets() {
        let root = root("reject");
        write_instance(&root, "old", json!({"id": "old"}));
        std::fs::create_dir_all(version_dir(&root, "taken")).unwrap();

        assert!(rename_instance(&root, "old", "a/b").await.is_err());
        assert!(rename_instance(&root, "old", "taken").await.is_err());
        assert!(version_dir(&root, "old").exists());

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[tokio::test]
    async fn delete_removes_the_instance_directory() {
        let root = root("delete");
        write_instance(&root, "old", json!({"id": "old"}));

        delete_instance(&root, "old").await.unwrap();
        assert!(!version_dir(&root, "old").exists());
        // 已经不存在时也视为成功。
        delete_instance(&root, "old").await.unwrap();

        std::fs::remove_dir_all(&root).unwrap();
    }
}
