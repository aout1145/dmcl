//! NeoForge 的安装（查询见 [`dmcl_mcmeta`]）。

use std::io::Cursor;
use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use dmcl_download::DownloadContext;
use dmcl_game::{ResolvedVersion, VersionRef};
use dmcl_mcmeta::layout::libraries_dir;
use dmcl_mcmeta::neoforge::{neoforge_installer_url, InstallProfile};
use dmcl_mcmeta::vanilla::{VersionJson, VersionJsonPatch};
use dmcl_task::{task_fn, Handle, StepId, TaskContext, TaskGraphBuilder};

use crate::archive::{has_entry, open_archive, read_entry};
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

/// 把 NeoForge 安装拆成「下载安装器 → 解析 → 解出库 / 下载依赖 → 跑 processor → 合并 JSON」。
///
/// 安装器只依赖版本目录已经创建；真正跑 processor 时再串上客户端 jar 与上一步的版本 JSON。
#[expect(clippy::too_many_arguments)]
pub(crate) fn add_neoforge_install(
    builder: &mut TaskGraphBuilder,
    step: StepId,
    options: Arc<InstallOptions>,
    downloads: Arc<DownloadContext>,
    created: Handle<VersionRef>,
    base_libraries: Handle<()>,
    client_jar: Handle<()>,
    previous: Handle<ResolvedVersion>,
    version_name: String,
    neoforge_version: String,
) -> Handle<VersionJsonPatch> {
    let installer = neoforge_installer(&neoforge_version);

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
            let neoforge_version = neoforge_version.clone();
            move |installer: Arc<Installer>, ctx: TaskContext| {
                let neoforge_version = neoforge_version.clone();
                async move { parse_installer(&installer, &neoforge_version, &ctx).await }
            }
        }),
    );

    let extracted = builder.add(
        step,
        "解出安装器库",
        plan.clone(),
        task_fn({
            let options = options.clone();
            move |plan: Arc<InstallerPlan>, ctx: TaskContext| {
                let options = options.clone();
                async move {
                    extract_neoforge_libraries(&options, &plan.installer.path, &ctx).await
                }
            }
        }),
    );

    let loader_deps = builder.add(
        step,
        "下载 NeoForge 依赖库",
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
                        "NeoForge 依赖库",
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
            let neoforge_version = neoforge_version.clone();
            move |(plan, previous, _extracted): (
                Arc<InstallerPlan>,
                Arc<ResolvedVersion>,
                Arc<()>,
            ),
                  ctx: TaskContext| {
                let options = options.clone();
                let downloads = downloads.clone();
                let version_name = version_name.clone();
                let neoforge_version = neoforge_version.clone();
                async move {
                    download_installer_dependencies(
                        &options,
                        &downloads,
                        &version_name,
                        &neoforge_version,
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
                let neoforge_version = neoforge_version.clone();
                async move {
                    run_installer_processors(
                        &options,
                        &version_name,
                        &neoforge_version,
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
                ctx.log(format!("已生成 NeoForge {} patch", plan.patch.version));
                Ok(plan.patch.clone())
            },
        ),
    )
}

fn neoforge_installer(version: &str) -> Installer {
    let (url, name) = neoforge_installer_url(version);
    let file_name = format!("{name}-{version}-installer.jar");
    Installer {
        url,
        path: installer_temp_path(version, &file_name),
        file_name,
    }
}

/// 解析安装器里的档案与自带版本 JSON，产出后续阶段使用的自有数据。
async fn parse_installer(
    installer: &Installer,
    neoforge_version: &str,
    ctx: &TaskContext,
) -> Result<InstallerPlan> {
    let bytes = tokio::fs::read(&installer.path).await?;
    let mut zip = open_archive(bytes)?;
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
    let client_lzma = if has_entry(&mut zip, "data/client.lzma") {
        Some(read_entry(&mut zip, "data/client.lzma")?)
    } else {
        None
    };
    let installer_json = read_installer_version_json(&mut zip)?;
    let patch = installer_patch("neoforge", neoforge_version, &installer_json);
    ctx.log(format!(
        "NeoForge {neoforge_version}：{} 个 processor",
        profile.processors.len()
    ));

    Ok(InstallerPlan {
        installer: installer.clone(),
        patch,
        libraries: profile.libraries,
        data,
        processors,
        client_lzma,
    })
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
    if has_entry(&mut zip, "maven/") {
        extract_maven_entries(&mut zip, &libraries, ctx).await?;
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
/// NeoForge 只有现代式安装器（spec 1）：版本 JSON 在 `install_profile.json` 的
/// `json`（通常是 `version.json`）指向的条目里。
fn read_installer_version_json(zip: &mut zip::ZipArchive<Cursor<Vec<u8>>>) -> Result<VersionJson> {
    let profile = read_install_profile(zip)?;
    let entry = profile.version_json_entry().to_string();
    let mut json: VersionJson = serde_json::from_slice(&read_entry(zip, &entry)?)?;
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
    fn installer_profile_urls_are_templatized() {
        let installer = zip_of(&[(
            "install_profile.json",
            br#"{"libraries":[{"name":"net.neoforged:srgutils:1.0.0",
                "downloads":{"artifact":{"url":"https://maven.neoforged.net/releases/net/neoforged/srgutils/1.0.0/srgutils-1.0.0.jar"}}}]}"#,
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
            Some("${NEOFORGE_MAVEN}/net/neoforged/srgutils/1.0.0/srgutils-1.0.0.jar")
        );
    }
}
