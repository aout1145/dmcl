//! Forge 的安装（查询见 [`dmcl_mcmeta`]）。

use std::collections::BTreeMap;
use std::io::Cursor;
use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use dmcl_download::DownloadContext;
use dmcl_game::{base_name, ResolvedVersion, VersionRef};
use dmcl_mcmeta::forge::{forge_installer_url, InstallProfile};
use dmcl_mcmeta::layout::libraries_dir;
use dmcl_mcmeta::maven::lib_name_to_path;
use dmcl_mcmeta::vanilla::{VersionJson, VersionJsonPatch};
use dmcl_task::{task_fn, Handle, StepId, TaskContext, TaskGraphBuilder};

use crate::archive::{has_entry, open_archive, read_entry};
use crate::io::write_safe;
use crate::loader::xforge::{
    download_installer, download_installer_dependencies, download_loader_libraries,
    extract_maven_entries, installer_patch, installer_temp_path, run_installer_processors,
    Installer, InstallerDeps, InstallerPlan, ProcessorPlan,
};
use crate::options::InstallOptions;

type ProcessorInput = (
    Arc<InstallerPlan>,
    Arc<InstallerDeps>,
    Arc<()>,
    Arc<()>,
    Arc<()>,
);

/// 把 Forge 安装拆成「下载安装器 → 解析 → 解出库 / 下载依赖 → 跑 processor → 合并 JSON」。
///
/// 安装器只依赖版本目录已经创建；真正跑 processor 时再串上客户端 jar 与上一步的版本 JSON。
#[expect(clippy::too_many_arguments)]
pub(crate) fn add_forge_install(
    builder: &mut TaskGraphBuilder,
    step: StepId,
    options: Arc<InstallOptions>,
    downloads: Arc<DownloadContext>,
    created: Handle<VersionRef>,
    base_libraries: Handle<()>,
    client_jar: Handle<()>,
    previous: Handle<ResolvedVersion>,
    version_name: String,
    forge_version: String,
) -> Handle<VersionJsonPatch> {
    let installer = forge_installer(&forge_version);

    let downloaded = builder.add(
        step,
        "下载安装器",
        created,
        task_fn({
            let options = options.clone();
            let downloads = downloads.clone();
            move |_: Arc<VersionRef>, ctx: TaskContext| {
                let options = options.clone();
                let downloads = downloads.clone();
                let installer = installer.clone();
                async move {
                    download_installer(&options, &downloads, &installer, &ctx).await?;
                    Ok(installer)
                }
            }
        }),
    );

    let plan = builder.add(
        step,
        "解析安装器",
        downloaded,
        task_fn({
            let forge_version = forge_version.clone();
            move |installer: Arc<Installer>, ctx: TaskContext| {
                let forge_version = forge_version.clone();
                async move { parse_installer(&installer, &forge_version, &ctx).await }
            }
        }),
    );

    let extracted = builder.add(
        step,
        "解出安装器库",
        plan.clone(),
        task_fn({
            let options = options.clone();
            let forge_version = forge_version.clone();
            move |plan: Arc<InstallerPlan>, ctx: TaskContext| {
                let options = options.clone();
                let forge_version = forge_version.clone();
                async move {
                    extract_forge_libraries(&options, &forge_version, &plan.installer.path, &ctx)
                        .await
                }
            }
        }),
    );

    let loader_deps = builder.add(
        step,
        "下载 Forge 依赖库",
        (plan.clone(), previous.clone(), extracted.clone()),
        task_fn({
            let options = options.clone();
            let downloads = downloads.clone();
            move |(plan, previous, _extracted): (
                Arc<InstallerPlan>,
                Arc<ResolvedVersion>,
                Arc<()>,
            ),
                  ctx: TaskContext| {
                let options = options.clone();
                let downloads = downloads.clone();
                async move {
                    download_loader_libraries(
                        &options,
                        &downloads,
                        &previous,
                        &plan,
                        "Forge 依赖库",
                        &ctx,
                    )
                    .await
                }
            }
        }),
    );

    let deps = builder.add(
        step,
        "下载安装器依赖",
        (plan.clone(), previous.clone(), extracted.clone()),
        task_fn({
            let options = options.clone();
            let version_name = version_name.clone();
            let forge_version = forge_version.clone();
            move |(plan, previous, _extracted): (
                Arc<InstallerPlan>,
                Arc<ResolvedVersion>,
                Arc<()>,
            ),
                  ctx: TaskContext| {
                let options = options.clone();
                let downloads = downloads.clone();
                let version_name = version_name.clone();
                let forge_version = forge_version.clone();
                async move {
                    download_installer_dependencies(
                        &options,
                        &downloads,
                        &version_name,
                        &forge_version,
                        step,
                        &previous,
                        &plan,
                        &ctx,
                    )
                    .await
                }
            }
        }),
    );

    let processed = builder.add(
        step,
        "运行安装器",
        (plan.clone(), deps, extracted, client_jar, base_libraries),
        task_fn({
            move |(plan, deps, _extracted, _client_jar, _base_libraries): ProcessorInput,
                  ctx: TaskContext| {
                let options = options.clone();
                let version_name = version_name.clone();
                let forge_version = forge_version.clone();
                async move {
                    run_installer_processors(
                        &options,
                        &version_name,
                        &forge_version,
                        &plan,
                        &deps,
                        &ctx,
                    )
                    .await
                }
            }
        }),
    );

    builder.add(
        step,
        "生成 patch",
        (loader_deps, plan, processed),
        task_fn(
            |(_loader_deps, plan, _): (Arc<()>, Arc<InstallerPlan>, Arc<()>),
             ctx: TaskContext| async move {
                ctx.log(format!("已生成 Forge {} patch", plan.patch.version));
                Ok(plan.patch.clone())
            },
        ),
    )
}

fn forge_installer(version: &str) -> Installer {
    let file_name = format!("forge-{version}-installer.jar");
    Installer {
        url: forge_installer_url(version),
        path: installer_temp_path(version, &file_name),
        file_name,
    }
}

/// 解析安装器里的档案与自带版本 JSON，产出后续阶段使用的自有数据。
async fn parse_installer(
    installer: &Installer,
    forge_version: &str,
    ctx: &TaskContext,
) -> Result<InstallerPlan> {
    let bytes = tokio::fs::read(&installer.path).await?;
    let mut zip = open_archive(bytes)?;

    // 老式安装器没有 `data/client.lzma`，不需要跑 processor，也就不需要解析档案。
    let client_lzma = if has_entry(&mut zip, "data/client.lzma") {
        Some(read_entry(&mut zip, "data/client.lzma")?)
    } else {
        None
    };
    let (libraries, data, processors) = if client_lzma.is_some() {
        let profile = read_install_profile(&mut zip)?;
        let data = profile
            .data
            .iter()
            .filter_map(|(key, entry)| entry.client().map(|value| (key.clone(), value.to_string())))
            .collect();
        let processors = profile
            .processors
            .iter()
            .filter(|step| step.is_client_side())
            .map(|step| ProcessorPlan {
                jar: step.jar.clone(),
                classpath: step.classpath.clone(),
                args: step.args.clone(),
                outputs: step.outputs.clone(),
            })
            .collect();
        (profile.libraries, data, processors)
    } else {
        (Vec::new(), BTreeMap::new(), Vec::new())
    };
    let installer_json = read_installer_version_json(&mut zip)?;
    let patch = installer_patch("forge", forge_version, &installer_json);
    ctx.log(format!(
        "Forge {forge_version}：{} 个 processor",
        processors.len()
    ));

    Ok(InstallerPlan {
        installer: installer.clone(),
        patch,
        libraries,
        data,
        processors,
        client_lzma,
    })
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

    if has_entry(&mut zip, "maven/") {
        extract_maven_entries(&mut zip, &libraries, ctx).await?;
    } else if !has_entry(&mut zip, "version.json") {
        // 老版 Forge：安装包根目录下只有一个 install_profile.json
        let profile = read_install_profile(&mut zip)?;
        let entry = profile.file_path().unwrap_or("").to_string();
        let bytes = read_entry(&mut zip, &entry)?;
        let target = libraries.join(lib_name_to_path(&format!(
            "net.minecraftforge:forge:{forge_version}"
        )));
        write_safe(&target, &bytes).await?;
        ctx.log(format!("解出 {}", base_name(&target)));
    }
    Ok(())
}

fn read_install_profile(zip: &mut zip::ZipArchive<Cursor<Vec<u8>>>) -> Result<InstallProfile> {
    let bytes = read_entry(zip, "install_profile.json")?;
    let mut profile: InstallProfile = serde_json::from_slice(&bytes)?;
    // 安装包里的库地址是具体 url，归一成模板后才能按镜像展开候选。
    profile.templatize_urls();
    Ok(profile)
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

    let mut json: VersionJson = if has_entry(zip, &version_entry) {
        serde_json::from_slice(&read_entry(zip, &version_entry)?)?
    } else if let Some(version_info) = profile.version_info {
        // 经典安装器：profile 就内联在 versionInfo 里，且地址是 https。
        *version_info
    } else {
        let inner = read_entry(zip, &inner_entry)?;
        let mut inner_zip = open_archive(inner)?;
        serde_json::from_slice(&read_entry(&mut inner_zip, "version.json")?)?
    };
    // 安装包里的地址是具体 url，归一到模板后写盘时才能再解析成官方地址
    json.templatize_urls();
    Ok(json)
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn installer_profile_urls_are_templatized() {
        let installer = zip_of(&[(
            "install_profile.json",
            br#"{"libraries":[{"name":"net.minecraftforge:forge:1.20.1",
                "downloads":{"artifact":{"url":"https://maven.minecraftforge.net/net/minecraftforge/forge/1.20.1/forge-1.20.1.jar"}}}]}"#,
        )]);
        let mut zip = open_archive(installer).unwrap();
        let profile = read_install_profile(&mut zip).unwrap();
        assert_eq!(
            profile.libraries[0]
                .downloads
                .as_ref()
                .unwrap()
                .artifact
                .as_ref()
                .unwrap()
                .url
                .as_deref(),
            Some("${FORGE_MAVEN}/net/minecraftforge/forge/1.20.1/forge-1.20.1.jar")
        );
    }
}
