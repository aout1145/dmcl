//! NeoForge 的安装（查询见 [`dmcl_mcmeta`]）。

use std::collections::BTreeMap;
use std::io::Cursor;
use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use dmcl_mcmeta::layout::{libraries_dir, read_version_json, version_json_path};
use dmcl_mcmeta::neoforge::{neoforge_installer_url, InstallProfile};
use dmcl_mcmeta::vanilla::VersionJson;
use dmcl_task::{task_fn_io, TaskChain, TaskContext};

use crate::io::write_json_safe;
use crate::loader::common::finish_version_json;
use crate::loader::xforge::{
    download_installer, extract_maven_entries, installer_temp_path, run_installer_processors,
    Installer, ProcessorSpec, ProfileInputs,
};
use crate::options::InstallOptions;
use crate::zip::{open_archive, zip_has_entry, zip_read_entry};

/// 顺序跑各阶段（薄封装）。
pub async fn install_neoforge(
    options: &InstallOptions,
    version_name: &str,
    forge_version: &str,
    ctx: &TaskContext,
) -> Result<()> {
    let installer = neoforge_installer(forge_version);
    download_installer(options, &installer, ctx).await?;
    extract_neoforge_libraries(options, &installer.path, ctx).await?;
    run_neoforge_installer(options, version_name, forge_version, &installer.path, ctx).await?;
    write_installer_version_json(options, version_name, &installer.path).await
}

fn neoforge_installer(version: &str) -> Installer {
    let (url, name) = neoforge_installer_url(version);
    let file_name = format!("{name}-{version}-installer.jar");
    Installer {
        version: version.to_string(),
        url,
        path: installer_temp_path(version, &file_name),
        file_name,
    }
}

/// 解出安装器里 `maven/` 下的文件。
async fn extract_neoforge_libraries(
    options: &InstallOptions,
    installer: &Path,
    ctx: &TaskContext,
) -> Result<()> {
    let libraries = libraries_dir(&options.root_path);
    let bytes = tokio::fs::read(installer).await?;
    let mut zip = open_archive(bytes)?;
    if zip_has_entry(&mut zip, "maven/") {
        extract_maven_entries(&mut zip, &libraries, ctx).await?;
    }
    Ok(())
}

/// 跑安装器 processor（NeoForge 没有 Forge 那条 `data/client.lzma` 判断）。
async fn run_neoforge_installer(
    options: &InstallOptions,
    version_name: &str,
    forge_version: &str,
    installer: &Path,
    ctx: &TaskContext,
) -> Result<()> {
    let bytes = tokio::fs::read(installer).await?;
    let mut zip = open_archive(bytes)?;
    let profile = read_install_profile(&mut zip)?;
    let data: BTreeMap<String, String> = profile
        .data
        .iter()
        .filter_map(|(key, entry)| entry.client().map(|value| (key.clone(), value.to_string())))
        .collect();
    let processors: Vec<ProcessorSpec<'_>> = profile
        .processors
        .iter()
        .filter(|step| step.is_client_side())
        .map(|step| ProcessorSpec {
            jar: &step.jar,
            classpath: &step.classpath,
            args: &step.args,
            outputs: step.outputs.as_ref(),
        })
        .collect();
    let inputs = ProfileInputs {
        libraries: &profile.libraries,
        data: &data,
        processors: &processors,
    };
    run_installer_processors(options, version_name, forge_version, inputs, &mut zip, ctx).await
}

fn read_install_profile(zip: &mut zip::ZipArchive<Cursor<Vec<u8>>>) -> Result<InstallProfile> {
    let bytes = zip_read_entry(zip, "install_profile.json")?;
    Ok(serde_json::from_slice(&bytes)?)
}

/// 从安装包里取出它自带的版本 JSON。
///
/// NeoForge 只有现代式安装器（spec 1）：版本 JSON 在 `install_profile.json` 的
/// `json`（通常是 `version.json`）指向的条目里。
fn read_installer_version_json(zip: &mut zip::ZipArchive<Cursor<Vec<u8>>>) -> Result<VersionJson> {
    let profile = read_install_profile(zip)?;
    let entry = profile.version_json_entry().to_string();
    let mut json: VersionJson = serde_json::from_slice(&zip_read_entry(zip, &entry)?)?;
    // 安装包里的地址是具体 url，归一到模板后写盘时才能再解析成官方地址
    json.templatize_urls();
    Ok(json)
}

/// 把安装器自带的版本 JSON 合并进本地版本 JSON 并写盘。
async fn write_installer_version_json(
    options: &InstallOptions,
    version_name: &str,
    installer: &Path,
) -> Result<()> {
    let path = version_json_path(&options.root_path, version_name);
    let mut mc_json = read_version_json(&path).await?;

    let bytes = tokio::fs::read(installer).await?;
    let mut zip = open_archive(bytes)?;
    let installer_json = read_installer_version_json(&mut zip)?;
    mc_json.merge(&installer_json)?;
    mc_json.extra.remove("_comment_");
    finish_version_json(&mut mc_json, version_name, true);

    write_json_safe(&path, &mc_json).await
}

/// 安装 NeoForge 的任务链。加载器版本由调用者给定（`dmcl-install` 不查最新）。
pub fn install_neoforge_task(
    options: Arc<InstallOptions>,
    version_name: String,
    forge_version: String,
) -> TaskChain<(), ()> {
    TaskChain::new(format!("安装 NeoForge：{version_name}"))
        .then(
            task_fn_io("下载 NeoForge 安装器", {
                let options = options.clone();
                move |_: (), ctx: TaskContext| {
                    let options = options.clone();
                    let installer = neoforge_installer(&forge_version);
                    async move {
                        download_installer(&options, &installer, &ctx).await?;
                        Ok(installer)
                    }
                }
            }),
            "下载 NeoForge 安装器",
        )
        .then(
            task_fn_io("解出安装器内的库", {
                let options = options.clone();
                move |installer: Installer, ctx: TaskContext| {
                    let options = options.clone();
                    async move {
                        extract_neoforge_libraries(&options, &installer.path, &ctx).await?;
                        Ok(installer)
                    }
                }
            }),
            "解出安装器内的库",
        )
        .then(
            task_fn_io("运行安装器", {
                let options = options.clone();
                let version_name = version_name.clone();
                move |installer: Installer, ctx: TaskContext| {
                    let options = options.clone();
                    let version_name = version_name.clone();
                    async move {
                        run_neoforge_installer(
                            &options,
                            &version_name,
                            &installer.version,
                            &installer.path,
                            &ctx,
                        )
                        .await?;
                        Ok(installer)
                    }
                }
            }),
            "运行安装器",
        )
        .then(
            task_fn_io("写入版本元数据", {
                let options = options.clone();
                let version_name = version_name.clone();
                move |installer: Installer, _ctx: TaskContext| {
                    let options = options.clone();
                    let version_name = version_name.clone();
                    async move {
                        write_installer_version_json(&options, &version_name, &installer.path).await
                    }
                }
            }),
            "写入版本元数据",
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use dmcl_mcmeta::source::{DownloadSource, Mirror};

    fn options() -> Arc<InstallOptions> {
        Arc::new(InstallOptions::new(
            "/tmp/.minecraft",
            DownloadSource::new(Mirror::Official),
        ))
    }

    /// 把若干条目打成内存 zip。
    fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buffer = Vec::new();
        {
            let mut writer = zip::ZipWriter::new(Cursor::new(&mut buffer));
            for (name, data) in files {
                let options: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
                writer.start_file(*name, options).unwrap();
                std::io::Write::write_all(&mut writer, data).unwrap();
            }
            writer.finish().unwrap();
        }
        buffer
    }

    #[test]
    fn installer_version_json_uses_the_documented_entry() {
        // `json` 指向包内的 version.json
        let installer = zip_of(&[
            (
                "install_profile.json",
                br#"{"spec":1,"json":"/version.json"}"#,
            ),
            ("version.json", br#"{"id":"modern","mainClass":"m"}"#),
        ]);
        let mut zip = open_archive(installer).unwrap();
        assert_eq!(
            read_installer_version_json(&mut zip).unwrap().id.as_deref(),
            Some("modern")
        );

        // `json` 指向别的位置时也按它来
        let custom = zip_of(&[
            (
                "install_profile.json",
                br#"{"spec":1,"json":"/profile/neoforge.json"}"#,
            ),
            (
                "profile/neoforge.json",
                br#"{"id":"custom","mainClass":"m"}"#,
            ),
        ]);
        let mut zip = open_archive(custom).unwrap();
        assert_eq!(
            read_installer_version_json(&mut zip).unwrap().id.as_deref(),
            Some("custom")
        );
    }

    #[test]
    fn neoforge_install_has_one_step_per_phase() {
        let chain = install_neoforge_task(options(), "1.20.1".into(), "47.1.0".into());
        let names: Vec<_> = chain.step_names().collect();
        assert_eq!(
            names,
            [
                "下载 NeoForge 安装器",
                "解出安装器内的库",
                "运行安装器",
                "写入版本元数据",
            ]
        );
    }
}
