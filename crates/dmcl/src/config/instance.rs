//! 实例 `dmcl.json` 的路径、`instance` 段与分类。
//!
//! 实例配置除了启动覆盖 `launch`，还可以有独立的 `instance` 段。分类只由
//! `instance.category` 决定：值就是分类名，缺省 / 空白表示未分类。清空分类用
//! [`clear_instance_category`]，它只删这个键，不影响 `instance` 段的其它字段。
//!
//! ```json
//! {
//!   "instance": { "category": "整合包" },
//!   "launch": { "max_memory": 4096 }
//! }
//! ```

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde_json::{json, Map, Value};

/// 实例配置文件路径：`<root>/versions/<version_name>/dmcl.json`。
pub fn instance_config_path(root: &Path, version_name: &str) -> PathBuf {
    dmcl_mcmeta::layout::version_dir(root, version_name).join("dmcl.json")
}

/// 分类名归一化：去掉首尾空白，只剩空白时视为未分类。
pub fn normalize_category(category: Option<String>) -> Option<String> {
    category
        .map(|category| category.trim().to_string())
        .filter(|category| !category.is_empty())
}

/// 异步读取一个实例的分类；文件不存在、无法解析或字段类型不对时按未分类处理。
pub async fn read_instance_category(root: &Path, version_name: &str) -> Option<String> {
    match tokio::fs::read_to_string(instance_config_path(root, version_name)).await {
        Ok(text) => category_from_text(&text),
        Err(_) => None,
    }
}

/// [`read_instance_category`] 的同步版本，给 GTK 初始化这类不能等待 future 的地方用。
pub fn read_instance_category_sync(root: &Path, version_name: &str) -> Option<String> {
    std::fs::read_to_string(instance_config_path(root, version_name))
        .ok()
        .and_then(|text| category_from_text(&text))
}

/// 从整份实例配置文本里取 `instance.category`。
///
/// 只读取分类，不校验 `launch`：启动配置的问题由启动流程统一报告，主页列表不因为
/// 一个坏字段就把整个分类信息当作启动错误。
fn category_from_text(text: &str) -> Option<String> {
    let document: Value = serde_json::from_str(text).ok()?;
    let category = document.get("instance")?.get("category")?.as_str()?;
    normalize_category(Some(category.to_string()))
}

/// 把分类写进实例的 `dmcl.json`，保留文件里已有的其它配置。
///
/// 分类为空时不写文件：缺省字段本身就是「未分类」。
pub async fn write_instance_category(
    root: &Path,
    version_name: &str,
    category: &str,
) -> Result<()> {
    let Some(category) = normalize_category(Some(category.to_string())) else {
        return Ok(());
    };

    edit_instance_document(root, version_name, move |path, document| {
        let instance = document.entry("instance").or_insert_with(|| json!({}));
        let Some(instance) = instance.as_object_mut() else {
            bail!("实例配置的「instance」不是 JSON 对象：{}", path.display());
        };
        instance.insert("category".to_string(), Value::String(category));
        Ok(())
    })
    .await
}

/// 清除实例分类：删掉 `instance.category`，`instance` 段空了就整段删除。
///
/// 文件不存在或本来没有分类时不写盘。
pub async fn clear_instance_category(root: &Path, version_name: &str) -> Result<()> {
    let path = instance_config_path(root, version_name);
    match tokio::fs::read_to_string(&path).await {
        Ok(text) => {
            let document: Value = serde_json::from_str(&text)
                .with_context(|| format!("无法解析实例配置 {}", path.display()))?;
            let has_category = document
                .get("instance")
                .and_then(Value::as_object)
                .is_some_and(|instance| instance.contains_key("category"));
            if !has_category {
                return Ok(());
            }
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => {
            return Err(err).with_context(|| format!("无法读取实例配置 {}", path.display()))
        }
    }

    edit_instance_document(root, version_name, move |path, document| {
        let Some(instance) = document.get_mut("instance") else {
            return Ok(());
        };
        let Some(instance) = instance.as_object_mut() else {
            bail!("实例配置的「instance」不是 JSON 对象：{}", path.display());
        };
        instance.remove("category");
        if instance.is_empty() {
            document.remove("instance");
        }
        Ok(())
    })
    .await
}

/// 按字段名修改实例 `dmcl.json` 的 `launch` 覆盖层。
///
/// `set` 里的字段写入、`remove` 里的字段删除，其它 `launch` 字段和整份文档的其它段原样
/// 保留；`launch` 变空时删掉整个段。字段名由调用方（GUI）决定。
pub async fn write_instance_launch(
    root: &Path,
    version_name: &str,
    set: Map<String, Value>,
    remove: &[&str],
) -> Result<()> {
    if set.is_empty() && remove.is_empty() {
        return Ok(());
    }

    edit_instance_document(root, version_name, move |path, document| {
        if document.get("launch").is_none() {
            if set.is_empty() {
                return Ok(());
            }
            document.insert("launch".to_string(), json!({}));
        }

        let empty = {
            let Some(launch) = document.get_mut("launch").and_then(Value::as_object_mut) else {
                bail!("实例配置的「launch」不是 JSON 对象：{}", path.display());
            };
            for key in remove {
                launch.remove(*key);
            }
            for (key, value) in set {
                launch.insert(key, value);
            }
            launch.is_empty()
        };
        if empty {
            document.remove("launch");
        }
        Ok(())
    })
    .await
}

/// 读取、修改并原子写回实例 `dmcl.json`；`edit` 只负责改文档内容。
async fn edit_instance_document(
    root: &Path,
    version_name: &str,
    edit: impl FnOnce(&Path, &mut Map<String, Value>) -> Result<()>,
) -> Result<()> {
    let path = instance_config_path(root, version_name);
    let value = match tokio::fs::read_to_string(&path).await {
        Ok(text) => serde_json::from_str::<Value>(&text)
            .with_context(|| format!("无法解析实例配置 {}", path.display()))?,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => json!({}),
        Err(err) => {
            return Err(err).with_context(|| format!("无法读取实例配置 {}", path.display()))
        }
    };
    let Value::Object(mut document) = value else {
        bail!("实例配置的顶层不是 JSON 对象：{}", path.display());
    };
    edit(&path, &mut document)?;

    if let Some(parent) = path.parent() {
        if !tokio::fs::metadata(parent)
            .await
            .is_ok_and(|metadata| metadata.is_dir())
        {
            bail!("实例目录不存在：{}", parent.display());
        }
    }
    let text = format!("{}\n", serde_json::to_string_pretty(&document)?);
    let temp = path.with_file_name(format!(".dmcl-{}.json.tmp", std::process::id()));
    let result = async {
        tokio::fs::write(&temp, text).await?;
        tokio::fs::rename(&temp, &path).await
    }
    .await;
    if let Err(err) = result {
        let _ = tokio::fs::remove_file(&temp).await;
        return Err(err).with_context(|| format!("无法写入实例配置 {}", path.display()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "dmcl-instance-category-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("versions").join("1.20.1")).unwrap();
        root
    }

    #[test]
    fn category_is_trimmed_and_blank_means_uncategorized() {
        assert_eq!(
            normalize_category(Some("  整合包  ".to_string())),
            Some("整合包".to_string())
        );
        assert_eq!(normalize_category(Some("   ".to_string())), None);
        assert_eq!(normalize_category(None), None);
    }

    #[tokio::test]
    async fn reads_category_from_the_instance_section_only() {
        let root = temp_root("read");
        let path = instance_config_path(&root, "1.20.1");
        std::fs::write(
            &path,
            r#"{"instance":{"category":" 冒险 "},"launch":{"max_memory":4096}}"#,
        )
        .unwrap();
        assert_eq!(
            read_instance_category(&root, "1.20.1").await,
            Some("冒险".to_string())
        );

        std::fs::write(&path, r#"{"launch":{"category":"不是这里"}}"#).unwrap();
        assert_eq!(read_instance_category(&root, "1.20.1").await, None);

        std::fs::write(&path, r#"{"instance":{"category":42}}"#).unwrap();
        assert_eq!(read_instance_category(&root, "1.20.1").await, None);

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[tokio::test]
    async fn write_preserves_launch_and_is_idempotent() {
        let root = temp_root("write");
        let path = instance_config_path(&root, "1.20.1");
        std::fs::write(&path, r#"{"launch":{"max_memory":4096}}"#).unwrap();

        write_instance_category(&root, "1.20.1", " 整合包 ")
            .await
            .unwrap();
        let document: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(document["instance"]["category"], "整合包");
        assert_eq!(document["launch"]["max_memory"], 4096);

        write_instance_category(&root, "1.20.1", "冒险")
            .await
            .unwrap();
        let document: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(document["instance"]["category"], "冒险");
        assert_eq!(document["launch"]["max_memory"], 4096);

        // 空分类不写文件，也不改写已有内容。
        let before = std::fs::read_to_string(&path).unwrap();
        write_instance_category(&root, "1.20.1", "  ")
            .await
            .unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[tokio::test]
    async fn clear_removes_only_the_category_and_keeps_other_sections() {
        let root = temp_root("clear");
        let path = instance_config_path(&root, "1.20.1");
        std::fs::write(
            &path,
            r#"{"instance":{"category":"整合包"},"launch":{"max_memory":4096}}"#,
        )
        .unwrap();

        clear_instance_category(&root, "1.20.1").await.unwrap();
        let document: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(document.get("instance").is_none());
        assert_eq!(document["launch"]["max_memory"], 4096);

        // 再清一次不碰文件。
        let before = std::fs::read_to_string(&path).unwrap();
        clear_instance_category(&root, "1.20.1").await.unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[tokio::test]
    async fn clear_does_not_create_a_missing_config() {
        let root = temp_root("clear-missing");
        let path = instance_config_path(&root, "1.20.1");
        clear_instance_category(&root, "1.20.1").await.unwrap();
        assert!(!path.exists());

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[tokio::test]
    async fn write_refuses_to_recreate_a_missing_instance_directory() {
        let root = temp_root("write-missing");
        let name = "gone";
        assert!(write_instance_category(&root, name, "整合包")
            .await
            .is_err());
        assert!(!dmcl_mcmeta::layout::version_dir(&root, name).exists());

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[tokio::test]
    async fn write_launch_preserves_other_sections_and_drops_empty_launch() {
        let root = temp_root("launch");
        let path = instance_config_path(&root, "1.20.1");
        std::fs::write(
            &path,
            r#"{
                "instance": {"category": "整合包"},
                "launch": {"max_memory": 4096, "window_width": 1000},
                "download": {"max_connections": 8}
            }"#,
        )
        .unwrap();

        let mut set = Map::new();
        set.insert("window_height".to_string(), json!(720));
        write_instance_launch(&root, "1.20.1", set, &["max_memory"])
            .await
            .unwrap();

        let document: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(document["instance"]["category"], "整合包");
        assert_eq!(document["download"]["max_connections"], 8);
        assert_eq!(document["launch"]["window_width"], 1000);
        assert_eq!(document["launch"]["window_height"], 720);
        assert!(document["launch"].get("max_memory").is_none());

        // 删光剩余字段后 launch 段消失，其它段保留。
        write_instance_launch(
            &root,
            "1.20.1",
            Map::new(),
            &["window_width", "window_height"],
        )
        .await
        .unwrap();
        let document: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(document.get("launch").is_none());
        assert_eq!(document["instance"]["category"], "整合包");

        // 没有可写内容时不碰文件。
        let before = std::fs::read_to_string(&path).unwrap();
        write_instance_launch(&root, "1.20.1", Map::new(), &[])
            .await
            .unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[tokio::test]
    async fn write_launch_refuses_a_non_object_launch_section() {
        let root = temp_root("launch-bad");
        let path = instance_config_path(&root, "1.20.1");
        std::fs::write(&path, r#"{"launch": 1}"#).unwrap();

        let mut set = Map::new();
        set.insert("max_memory".to_string(), json!(4096));
        assert!(write_instance_launch(&root, "1.20.1", set, &[])
            .await
            .is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), r#"{"launch": 1}"#);

        std::fs::remove_dir_all(&root).unwrap();
    }
}
