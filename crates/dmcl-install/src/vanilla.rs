//! 原版：版本 JSON 落盘 + `launcher_profiles.json`。

use std::sync::Arc;

use anyhow::{Context, Result};
use dmcl_mcmeta::layout::{absolute, version_json_path};
use dmcl_mcmeta::source::Mirror;
use dmcl_task::{task_fn, TaskChain};

use crate::io::{write_json_safe, write_safe};
use crate::options::InstallOptions;

/// 新建 `.minecraft` 时写入的 `launcher_profiles.json`（与官方启动器一致的最小内容）。
pub const LAUNCHER_PROFILES_JSON: &str = r#"{"selectedProfile": "(Default)","profiles": {"(Default)": {"name": "(Default)"}},"clientToken": "88888888-8888-8888-8888-888888888888"}"#;

/// 确保 `launcher_profiles.json` 里存在 `(Default)` 档案、`selectedProfile` 与 `clientToken`。
///
/// * 文件不存在：写入 [`LAUNCHER_PROFILES_JSON`]；
/// * 文件存在：解析后**只补缺失的键**，用户已有的档案 / 设置原样保留；解析失败则不碰它。
pub async fn generate_launcher_profiles(root_path: impl AsRef<std::path::Path>) -> Result<()> {
    let path = absolute(root_path).join("launcher_profiles.json");
    let existing = match tokio::fs::read_to_string(&path).await {
        Ok(text) => Some(text),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => return Err(err).with_context(|| format!("无法读取 {}", path.display())),
    };

    let Some(existing) = existing else {
        return write_safe(&path, LAUNCHER_PROFILES_JSON.as_bytes()).await;
    };

    // 已有内容：只补缺失的键。解析不出来（被改坏了）就原样不动，别覆盖用户数据。
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(&existing) else {
        return Ok(());
    };
    let Some(object) = value.as_object_mut() else {
        return Ok(());
    };

    let mut changed = false;
    if !object.contains_key("selectedProfile") {
        object.insert(
            "selectedProfile".to_string(),
            serde_json::json!("(Default)"),
        );
        changed = true;
    }
    if !object.contains_key("clientToken") {
        object.insert(
            "clientToken".to_string(),
            serde_json::json!("88888888-8888-8888-8888-888888888888"),
        );
        changed = true;
    }
    let profiles = object
        .entry("profiles")
        .or_insert_with(|| serde_json::json!({}));
    if let Some(profiles) = profiles.as_object_mut() {
        if !profiles.contains_key("(Default)") {
            profiles.insert(
                "(Default)".to_string(),
                serde_json::json!({ "name": "(Default)" }),
            );
            changed = true;
        }
    }

    if changed {
        let text = serde_json::to_string(&value)?;
        write_safe(&path, text.as_bytes()).await?;
    }
    Ok(())
}

/// 在版本列表里找到 `version_id`，把它的版本 JSON 存成
/// `versions/<version_name>/<version_name>.json`。
///
/// 写盘前会把 URL 模板解析成**官方**地址（而不是当前镜像的地址），
/// 这样 `.minecraft` 里的文档与官方启动器保持一致，换镜像也不需要重写文件。
pub async fn download_vanilla(
    options: &InstallOptions,
    version_id: &str,
    version_name: &str,
) -> Result<()> {
    generate_launcher_profiles(&options.root_path).await?;

    let mut json = options.source.fetch_version_json_by_id(version_id).await?;
    json.id = Some(version_name.to_string());
    json.resolve_urls(Mirror::Official);

    write_json_safe(version_json_path(&options.root_path, version_name), &json).await
}

/// 取原版元数据的任务链（一个步骤）。
pub fn vanilla_metadata_task(
    options: Arc<InstallOptions>,
    version_id: String,
    version_name: String,
) -> TaskChain<(), ()> {
    TaskChain::new(format!("获取 {version_id} 的版本元数据")).then(
        task_fn(move |ctx| {
            let options = options.clone();
            let version_id = version_id.clone();
            let version_name = version_name.clone();
            async move {
                ctx.log(format!("查询版本列表并保存 {version_name}.json"));
                download_vanilla(&options, &version_id, &version_name).await?;
                ctx.log(format!("{version_name}.json 已写入"));
                Ok(())
            }
        }),
        "获取版本元数据",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn launcher_profiles_are_created_then_merged() {
        let root = std::env::temp_dir().join(format!("dmcl-profiles-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&root).await;
        tokio::fs::create_dir_all(&root).await.unwrap();
        let path = root.join("launcher_profiles.json");

        // 不存在 → 写最小内容
        generate_launcher_profiles(&root).await.unwrap();
        let text = tokio::fs::read_to_string(&path).await.unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["profiles"]["(Default)"]["name"], "(Default)");
        assert_eq!(value["selectedProfile"], "(Default)");

        // 已有用户内容 → 只补缺失键，保留其它
        tokio::fs::write(
            &path,
            r#"{"profiles":{"mine":{"name":"mine"}},"userSetting":1}"#,
        )
        .await
        .unwrap();
        generate_launcher_profiles(&root).await.unwrap();
        let text = tokio::fs::read_to_string(&path).await.unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["profiles"]["mine"]["name"], "mine");
        assert_eq!(value["userSetting"], 1);
        assert_eq!(value["profiles"]["(Default)"]["name"], "(Default)");
        assert_eq!(value["selectedProfile"], "(Default)");

        tokio::fs::remove_dir_all(&root).await.unwrap();
    }
}
