//! Forge / NeoForge 安装共用的内部工具。

use std::io::Cursor;
use std::path::{Path, PathBuf};

use anyhow::Result;
use dmcl_mcmeta::source::Mirror;
use dmcl_mcmeta::vanilla::VersionJson;
use dmcl_task::TaskContext;

use crate::files::download_planned;
use crate::io::write_safe;
use crate::options::InstallOptions;
use crate::plan::{base_name, PlannedFile};

/// 安装 Forge / NeoForge 时在步骤之间传递的中间值。
pub(crate) struct Installer {
    /// 安装器对应的加载器版本号。
    pub(crate) version: String,
    /// 安装器的下载地址。
    pub(crate) url: String,
    /// 安装器文件名。
    pub(crate) file_name: String,
    /// 安装器落地的临时路径。
    pub(crate) path: PathBuf,
}

/// 安装器临时路径：`<tmp>/dmcl-installer-<label>/<file_name>`。
pub(crate) fn installer_temp_path(label: &str, file_name: &str) -> PathBuf {
    std::env::temp_dir()
        .join(format!("dmcl-installer-{label}"))
        .join(file_name)
}

/// 下载安装器到临时路径。后续步骤按需重新打开，不把字节流长期攥在手里。
pub(crate) async fn download_installer(
    options: &InstallOptions,
    installer: &Installer,
    ctx: &TaskContext,
) -> Result<()> {
    download_planned(
        options,
        "安装器",
        vec![PlannedFile {
            url: installer.url.clone(),
            save_to: installer.path.clone(),
            checksum: None,
            size: None,
            name: installer.file_name.clone(),
        }],
        ctx,
    )
    .await
    .map(|_| ())
}

/// 安装包里 `maven/` 下的文件直接铺到 `libraries/`。
pub(crate) async fn extract_maven_entries(
    zip: &mut zip::ZipArchive<Cursor<Vec<u8>>>,
    libraries: &Path,
    ctx: &TaskContext,
) -> Result<()> {
    let count = zip.len();
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    for index in 0..count {
        let mut entry = zip.by_index(index)?;
        let name = entry.name().to_string();
        if !name.ends_with('/') && name.starts_with("maven/") {
            use std::io::Read;
            let mut buffer = Vec::new();
            entry.read_to_end(&mut buffer)?;
            files.push((name, buffer));
        }
    }
    for (name, buffer) in files {
        let target = libraries.join(&name["maven/".len()..]);
        write_safe(&target, &buffer).await?;
        ctx.log(format!("解出 {}", base_name(&name)));
    }
    Ok(())
}

/// 收尾：写盘前把 URL 模板解析回官方地址，并记下客户端 jar 名。
pub(crate) fn finish_version_json(json: &mut VersionJson, version_name: &str, own_jar: bool) {
    json.resolve_urls(Mirror::Official);
    json.inherits_from = None;
    json.id = Some(version_name.to_string());
    if own_jar {
        json.jar = Some(version_name.to_string());
    }
}
