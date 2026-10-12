//! 写文件的小助手。

use std::path::Path;

use anyhow::{Context, Result};
use dmcl_mcmeta::layout::absolute;

/// 先建父目录再写文件。
pub async fn write_safe(path: impl AsRef<Path>, context: &[u8]) -> Result<()> {
    let path = absolute(path);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.ok();
    }
    tokio::fs::write(&path, context)
        .await
        .with_context(|| format!("无法写入 {}", path.display()))?;
    Ok(())
}

/// 序列化 + 写文件（紧凑格式，不缩进）。
pub async fn write_json_safe<T: serde::Serialize>(path: impl AsRef<Path>, value: &T) -> Result<()> {
    let text = serde_json::to_string(value)?;
    write_safe(path, text.as_bytes()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn writing_creates_parent_directories() {
        let dir = std::env::temp_dir().join(format!("dmcl-install-io-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&dir).await;
        let path = dir.join("a/b/c.txt");

        write_safe(&path, b"hello").await.unwrap();
        assert_eq!(tokio::fs::read_to_string(&path).await.unwrap(), "hello");

        let json_path = dir.join("a/d.json");
        write_json_safe(&json_path, &serde_json::json!({"a": 1}))
            .await
            .unwrap();
        // 紧凑格式（没有多余空格）
        assert_eq!(
            tokio::fs::read_to_string(&json_path).await.unwrap(),
            r#"{"a":1}"#
        );

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }
}
