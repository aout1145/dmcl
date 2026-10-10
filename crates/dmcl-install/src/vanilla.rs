//! 原版：版本 JSON 落盘 + `launcher_profiles.json`。

use std::sync::Arc;

use anyhow::Result;
use dmcl_mcmeta::{layout::absolute, version_json_path, Mirror};
use dmcl_task::{task_fn, TaskChain};

use crate::io::{write_json_safe, write_safe};
use crate::options::InstallOptions;

/// 新建 `.minecraft` 时写入的 `launcher_profiles.json`
/// —— 与官方启动器 / 原实现逐字节一致。
pub const LAUNCHER_PROFILES_JSON: &str = r#"{"selectedProfile": "(Default)","profiles": {"(Default)": {"name": "(Default)"}},"clientToken": "88888888-8888-8888-8888-888888888888"}"#;

/// `generateLauncherProfiles` —— 文件不存在时才创建。
pub async fn generate_launcher_profiles(root_path: impl AsRef<std::path::Path>) -> Result<()> {
    let path = absolute(root_path).join("launcher_profiles.json");
    if tokio::fs::metadata(&path).await.is_err() {
        write_safe(&path, LAUNCHER_PROFILES_JSON.as_bytes()).await?;
    }
    Ok(())
}

/// `downloadVanilla` —— 在版本列表里找到 `version_id`，把它的版本 JSON 存成
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
        task_fn("获取版本元数据", move |ctx| {
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
