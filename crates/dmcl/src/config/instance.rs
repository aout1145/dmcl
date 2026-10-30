//! 实例 `dmcl.json` 的路径、`instance` 段与分类。
//!
//! 实例配置除了启动覆盖 `launch`，还可以有独立的 `instance` 段。分类只由
//! `instance.category` 决定：值就是分类名，缺省 / 空白表示未分类。
//!
//! ```json
//! {
//!   "instance": { "category": "整合包" },
//!   "launch": { "max_memory": 4096 }
//! }
//! ```

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

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
/// 分类为空时不写文件：缺省字段本身就是「未分类」。写入走同目录临时文件再重命名，
/// 避免读到半截 JSON。
pub async fn write_instance_category(
    root: &Path,
    version_name: &str,
    category: &str,
) -> Result<()> {
    let Some(category) = normalize_category(Some(category.to_string())) else {
        return Ok(());
    };

    let path = instance_config_path(root, version_name);
    let mut document = match tokio::fs::read_to_string(&path).await {
        Ok(text) => serde_json::from_str::<Value>(&text)
            .with_context(|| format!("无法解析实例配置 {}", path.display()))?,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => json!({}),
        Err(err) => {
            return Err(err).with_context(|| format!("无法读取实例配置 {}", path.display()))
        }
    };
    if !document.is_object() {
        bail!("实例配置的顶层不是 JSON 对象：{}", path.display());
    }

    let instance = document
        .as_object_mut()
        .expect("已确认顶层是对象")
        .entry("instance")
        .or_insert_with(|| json!({}));
    let Some(instance) = instance.as_object_mut() else {
        bail!("实例配置的「instance」不是 JSON 对象：{}", path.display());
    };
    instance.insert("category".to_string(), Value::String(category));

    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("无法创建版本目录 {}", parent.display()))?;
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
}
