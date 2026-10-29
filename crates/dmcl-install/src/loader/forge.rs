//! Forge 的安装（查询见 [`dmcl_mcmeta`]）。

use std::collections::BTreeMap;
use std::io::Cursor;
use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use dmcl_mcmeta::forge::{forge_installer_url, InstallProfile};
use dmcl_mcmeta::layout::{libraries_dir, read_version_json, version_json_path};
use dmcl_mcmeta::maven::lib_name_to_path;
use dmcl_mcmeta::vanilla::VersionJson;
use dmcl_task::{task_fn_io, TaskChain, TaskContext};

use crate::io::{write_json_safe, write_safe};
use crate::loader::common::finish_version_json;
use crate::loader::xforge::{
    download_installer, extract_maven_entries, installer_temp_path, run_installer_processors,
    Installer, ProcessorSpec, ProfileInputs,
};
use crate::options::InstallOptions;
use crate::plan::base_name;
use crate::zip::{open_archive, zip_has_entry, zip_read_entry};

/// 顺序跑各阶段（薄封装）。
pub async fn install_forge(
    options: &InstallOptions,
    version_name: &str,
    forge_version: &str,
    ctx: &TaskContext,
) -> Result<()> {
    let installer = forge_installer(forge_version);
    download_installer(options, &installer, ctx).await?;
    extract_forge_libraries(options, forge_version, &installer.path, ctx).await?;
    run_forge_installer(options, version_name, forge_version, &installer.path, ctx).await?;
    write_installer_version_json(options, version_name, &installer.path).await
}

fn forge_installer(version: &str) -> Installer {
    let file_name = format!("forge-{version}-installer.jar");
    Installer {
        version: version.to_string(),
        url: forge_installer_url(version),
        path: installer_temp_path(version, &file_name),
        file_name,
    }
}

/// 解出安装器里 `maven/` 下的文件；老版把单个 jar 直接铺进 `libraries/`。
async fn extract_forge_libraries(
    options: &InstallOptions,
    forge_version: &str,
    installer: &Path,
    ctx: &TaskContext,
) -> Result<()> {
    let libraries = libraries_dir(&options.root_path);
    let bytes = tokio::fs::read(installer).await?;
    let mut zip = open_archive(bytes)?;

    if zip_has_entry(&mut zip, "maven/") {
        extract_maven_entries(&mut zip, &libraries, ctx).await?;
    } else if !zip_has_entry(&mut zip, "version.json") {
        // 老版 Forge：安装包根目录下只有一个 install_profile.json
        let profile = read_install_profile(&mut zip)?;
        let entry = profile.file_path().unwrap_or("").to_string();
        let bytes = zip_read_entry(&mut zip, &entry)?;
        let target = libraries.join(lib_name_to_path(&format!(
            "net.minecraftforge:forge:{forge_version}"
        )));
        write_safe(&target, &bytes).await?;
        ctx.log(format!("解出 {}", base_name(&target)));
    }
    Ok(())
}

/// 1.13+ 跑安装器 processor 生成 forge-client.jar。
async fn run_forge_installer(
    options: &InstallOptions,
    version_name: &str,
    forge_version: &str,
    installer: &Path,
    ctx: &TaskContext,
) -> Result<()> {
    let bytes = tokio::fs::read(installer).await?;
    let mut zip = open_archive(bytes)?;
    if !zip_has_entry(&mut zip, "data/client.lzma") {
        return Ok(());
    }
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
/// 三代安装器把 profile 放在不同位置：
/// * 现代式（spec 0/1）：`install_profile.json` 的 `json`（通常 `version.json`）指向的条目；
/// * 经典式（≤1.11.2）：内联在 `install_profile.json` 的 `versionInfo` 里；
/// * 兜底：有些包只把它打在 `install.filePath` 指向的内嵌 jar 里。
fn read_installer_version_json(zip: &mut zip::ZipArchive<Cursor<Vec<u8>>>) -> Result<VersionJson> {
    let profile = read_install_profile(zip)?;
    let version_entry = profile.version_json_entry().to_string();
    let inner_entry = profile.file_path().unwrap_or("").to_string();

    let mut json: VersionJson = if zip_has_entry(zip, &version_entry) {
        serde_json::from_slice(&zip_read_entry(zip, &version_entry)?)?
    } else if let Some(version_info) = profile.version_info {
        // 经典安装器：profile 就内联在 versionInfo 里，且地址是 https。
        *version_info
    } else {
        let inner = zip_read_entry(zip, &inner_entry)?;
        let mut inner_zip = open_archive(inner)?;
        serde_json::from_slice(&zip_read_entry(&mut inner_zip, "version.json")?)?
    };
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

/// 安装 Forge 的任务链。加载器版本由调用者给定（`dmcl-install` 不查最新）。
pub fn install_forge_task(
    options: Arc<InstallOptions>,
    version_name: String,
    forge_version: String,
) -> TaskChain<(), ()> {
    TaskChain::new(format!("安装 Forge：{version_name}"))
        .then(
            task_fn_io({
                let options = options.clone();
                move |_: (), ctx: TaskContext| {
                    let options = options.clone();
                    let installer = forge_installer(&forge_version);
                    async move {
                        download_installer(&options, &installer, &ctx).await?;
                        Ok(installer)
                    }
                }
            }),
            "下载 Forge 安装器",
        )
        .then(
            task_fn_io({
                let options = options.clone();
                move |installer: Installer, ctx: TaskContext| {
                    let options = options.clone();
                    async move {
                        extract_forge_libraries(
                            &options,
                            &installer.version,
                            &installer.path,
                            &ctx,
                        )
                        .await?;
                        Ok(installer)
                    }
                }
            }),
            "解出安装器内的库",
        )
        .then(
            task_fn_io({
                let options = options.clone();
                let version_name = version_name.clone();
                move |installer: Installer, ctx: TaskContext| {
                    let options = options.clone();
                    let version_name = version_name.clone();
                    async move {
                        run_forge_installer(
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
            task_fn_io({
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
                let options: zip::write::FileOptions<'_, '_, ()> =
                    zip::write::FileOptions::default();
                writer.start_file(*name, options).unwrap();
                std::io::Write::write_all(&mut writer, data).unwrap();
            }
            writer.finish().unwrap();
        }
        buffer
    }

    #[test]
    fn installer_version_json_handles_all_three_layouts() {
        // 现代式：`json` 指向包内的 version.json
        let modern = zip_of(&[
            (
                "install_profile.json",
                br#"{"spec":1,"json":"/version.json"}"#,
            ),
            ("version.json", br#"{"id":"modern","mainClass":"m"}"#),
        ]);
        let mut zip = open_archive(modern).unwrap();
        assert_eq!(
            read_installer_version_json(&mut zip).unwrap().id.as_deref(),
            Some("modern")
        );

        // 经典式：profile 内联在 versionInfo 里，包内没有 version.json
        let classic = zip_of(&[(
            "install_profile.json",
            br#"{"install":{"filePath":"forge-universal.jar"},"versionInfo":{"id":"classic","mainClass":"m"}}"#,
        )]);
        let mut zip = open_archive(classic).unwrap();
        assert_eq!(
            read_installer_version_json(&mut zip).unwrap().id.as_deref(),
            Some("classic")
        );

        // 兜底：version.json 只在 install.filePath 指向的内嵌 jar 里
        let inner = zip_of(&[("version.json", br#"{"id":"inner","mainClass":"m"}"#)]);
        let outer = zip_of(&[
            (
                "install_profile.json",
                br#"{"install":{"filePath":"forge-universal.jar"}}"#,
            ),
            ("forge-universal.jar", &inner),
        ]);
        let mut zip = open_archive(outer).unwrap();
        assert_eq!(
            read_installer_version_json(&mut zip).unwrap().id.as_deref(),
            Some("inner")
        );
    }

    #[test]
    fn forge_install_has_one_step_per_phase() {
        let chain = install_forge_task(options(), "1.20.1".into(), "47.2.0".into());
        let names: Vec<_> = chain.step_names().collect();
        assert_eq!(
            names,
            [
                "下载 Forge 安装器",
                "解出安装器内的库",
                "运行安装器",
                "写入版本元数据",
            ]
        );
    }
}
