//! 调整已安装实例的加载器。
//!
//! 只处理由 Dacite 安装的实例：版本 JSON 的 `dmcl-patches[0]` 是未修改的原版快照，
//! 其后的 patch 描述加载器。更新时以该快照为基底重新应用一组加载器 patch，未变化的
//! 加载器直接复用已存 patch，新增或换版本的加载器重新安装；自定义（非加载器）patch 原样
//! 保留并先于加载器应用。

use std::path::Path;
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use dmcl_game::{ResolvedVersion, VersionRef};
use dmcl_mcmeta::layout::version_json_path;
use dmcl_mcmeta::vanilla::{VersionJson, VersionJsonPatch};
use dmcl_task::{task_fn, TaskGraph, TaskGraphBuilder};

use crate::io::write_json_safe;
use crate::loader::fabric::add_fabric_install;
use crate::loader::forge::add_forge_install;
use crate::loader::neoforge::add_neoforge_install;
use crate::loader::optifine::add_optifine_install;
use crate::options::InstallOptions;
use crate::spec::{
    download_client_jar_step, download_libraries_step, finish_version_json, InstallOutcome,
    LoaderKind, LoaderRequest,
};

/// 加载器的规范应用顺序：Forge 必须在 OptiFine 之前。
const LOADER_ORDER: [LoaderKind; 4] = [
    LoaderKind::Forge,
    LoaderKind::Fabric,
    LoaderKind::NeoForge,
    LoaderKind::OptiFine,
];

/// 一次加载器调整。
#[derive(Debug, Clone)]
pub struct LoaderUpdate {
    /// 已安装实例的版本名（`versions/<名称>`）。
    pub version_name: String,
    /// 目标加载器；`version` 与安装时使用的一致（Forge 是 `full_name`，OptiFine 不带
    /// Minecraft 前缀）。
    pub loaders: Vec<LoaderRequest>,
}

/// 更新任务里复用的已存 patch。
#[derive(Clone)]
struct ReusedPatches {
    /// 非加载器 patch，按原文件顺序保留。
    unknown: Vec<VersionJsonPatch>,
    /// 与目标一致、不需要重新安装的加载器 patch。
    loaders: Vec<(LoaderKind, VersionJsonPatch)>,
}

/// 构图前从版本 JSON 读出的更新计划。
struct UpdatePlan {
    base: ResolvedVersion,
    mc_version: String,
    has_forge: bool,
    reused: ReusedPatches,
    regenerate: Vec<LoaderRequest>,
}

/// 调整某个已安装实例的加载器，返回可直接运行的任务图。
///
/// 实例必须带 `dmcl-patches`，否则报错。版本 JSON 在构图时同步读取；运行期间只写
/// `versions/<名称>/<名称>.json`，下载的库作为共享缓存保留。
pub fn update_loaders_task(
    options: Arc<InstallOptions>,
    update: LoaderUpdate,
) -> Result<TaskGraph<InstallOutcome>> {
    let plan = plan_update(&options.root_path, &update)?;
    let mut builder = TaskGraphBuilder::new(format!("调整 {} 的加载器", update.version_name));
    let downloads =
        (!plan.regenerate.is_empty()).then(|| options.game.download_context(builder.progress()));

    let prepare = builder.step("准备");
    let base = builder.add(prepare, "读取原版快照", (), {
        let base = plan.base;
        task_fn(move |_: (), _ctx| {
            let base = base.clone();
            async move { Ok(base) }
        })
    });
    let reused = builder.add(prepare, "整理已有 patch", (), {
        let reused = plan.reused;
        task_fn(move |_: (), _ctx| {
            let reused = reused.clone();
            async move { Ok(reused) }
        })
    });

    // Forge / NeoForge 的 processor 需要原版库和客户端 jar。
    let needs_installer_files = plan
        .regenerate
        .iter()
        .any(|request| matches!(request.kind, LoaderKind::Forge | LoaderKind::NeoForge));
    let (client_jar, libraries, created) = if needs_installer_files {
        let step = builder.step("检查游戏文件");
        let downloads = downloads
            .as_ref()
            .expect("有加载器要安装时一定创建了下载上下文")
            .clone();
        let libraries = builder.add(
            step,
            "检查原版依赖库",
            base.clone(),
            download_libraries_step(options.clone(), downloads.clone()),
        );
        let client_jar = builder.add(
            step,
            "检查版本 jar",
            base.clone(),
            download_client_jar_step(options.clone(), downloads),
        );
        let created = builder.add(
            prepare,
            "准备版本目录",
            base.clone(),
            task_fn(
                |version: Arc<ResolvedVersion>, _ctx| async move { Ok(version.reference.clone()) },
            ),
        );
        (Some(client_jar), Some(libraries), Some(created))
    } else {
        (None, None, None)
    };

    let mut generated = Vec::new();
    for request in plan.regenerate {
        let step = builder.step(format!("安装 {}", request.kind.display_name()));
        let downloads = downloads
            .as_ref()
            .expect("有加载器要安装时一定创建了下载上下文")
            .clone();
        let mc_version = plan.mc_version.clone();
        let version_name = update.version_name.clone();
        let patch = match request.kind {
            LoaderKind::Fabric => add_fabric_install(
                &mut builder,
                step,
                options.clone(),
                downloads,
                base.clone(),
                mc_version,
                request.version,
            ),
            LoaderKind::Forge => add_forge_install(
                &mut builder,
                step,
                options.clone(),
                downloads,
                created.clone().expect("Forge 安装前已准备版本目录"),
                libraries.clone().expect("Forge 安装前已检查原版依赖库"),
                client_jar.clone().expect("Forge 安装前已检查版本 jar"),
                base.clone(),
                version_name,
                request.version,
            ),
            LoaderKind::NeoForge => add_neoforge_install(
                &mut builder,
                step,
                options.clone(),
                downloads,
                created.clone().expect("NeoForge 安装前已准备版本目录"),
                libraries.clone().expect("NeoForge 安装前已检查原版依赖库"),
                client_jar.clone().expect("NeoForge 安装前已检查版本 jar"),
                base.clone(),
                version_name,
                request.version,
            ),
            LoaderKind::OptiFine => add_optifine_install(
                &mut builder,
                step,
                options.clone(),
                downloads,
                mc_version,
                request.version,
                plan.has_forge,
            ),
        };
        generated.push(patch);
    }

    let finish = builder.step("完成");
    let version_name = update.version_name;
    let sink = builder.add(
        finish,
        "写入版本 JSON",
        (base, reused, generated),
        task_fn(
            move |(base, reused, generated): (
                Arc<ResolvedVersion>,
                Arc<ReusedPatches>,
                Vec<Arc<VersionJsonPatch>>,
            ),
                  ctx| {
                let version_name = version_name.clone();
                async move {
                    let patches = collect_patches(&reused, &generated)?;
                    let mut json = base.json.clone();
                    json.apply_patches(&patches)?;
                    finish_version_json(&mut json, &version_name);
                    write_json_safe(&base.reference.json_path(), &json).await?;
                    ctx.log(format!("加载器已更新（{} 个 patch）", patches.len()));
                    Ok(InstallOutcome {
                        version: base.reference.clone(),
                    })
                }
            },
        ),
    );
    builder.build(sink).map_err(Into::into)
}

/// 把复用与重新生成的 patch 合成最终应用序列：非加载器 patch 在前，加载器按固定顺序。
fn collect_patches(
    reused: &ReusedPatches,
    generated: &[Arc<VersionJsonPatch>],
) -> Result<Vec<VersionJsonPatch>> {
    let mut generated_kinds = Vec::new();
    for patch in generated {
        let kind = LoaderKind::from_patch_name(&patch.name)
            .ok_or_else(|| anyhow::anyhow!("加载器安装返回了无法识别的 patch：{}", patch.name))?;
        if generated_kinds.contains(&kind) {
            bail!("加载器安装返回了重复的 {} patch", kind.display_name());
        }
        generated_kinds.push(kind);
    }

    let mut patches = reused.unknown.clone();
    for kind in LOADER_ORDER {
        let patch = generated
            .iter()
            .find(|patch| LoaderKind::from_patch_name(&patch.name) == Some(kind))
            .map(|patch| (**patch).clone())
            .or_else(|| {
                reused
                    .loaders
                    .iter()
                    .find(|(reused_kind, _)| *reused_kind == kind)
                    .map(|(_, patch)| patch.clone())
            });
        if let Some(patch) = patch {
            patches.push(patch);
        }
    }
    Ok(patches)
}

/// 读取实例版本 JSON，决定复用哪些 patch、重新安装哪些加载器。
fn plan_update(root: &Path, update: &LoaderUpdate) -> Result<UpdatePlan> {
    let path = version_json_path(root, &update.version_name);
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("无法读取版本 JSON {}", path.display()))?;
    let mut json: VersionJson = serde_json::from_str(&text)
        .with_context(|| format!("无法解析版本 JSON {}", path.display()))?;
    json.templatize_urls();

    if json.dmcl_patches.is_empty() {
        bail!(
            "实例「{}」不是由 Dacite 创建的，无法调整加载器",
            update.version_name
        );
    }

    let mut base: VersionJson = serde_json::from_value(json.dmcl_patches[0].clone())
        .with_context(|| format!("无法解析 {} 的 dmcl-patches[0]", path.display()))?;
    base.templatize_urls();
    let mc_version = base
        .id
        .clone()
        .filter(|id| !id.is_empty())
        .ok_or_else(|| anyhow::anyhow!("无法从 dmcl-patches[0] 判断 Minecraft 版本"))?;

    let mut desired_kinds = Vec::new();
    for request in &update.loaders {
        if desired_kinds.contains(&request.kind) {
            bail!("加载器 {} 被重复指定", request.kind.display_name());
        }
        desired_kinds.push(request.kind);
    }

    let mut unknown = Vec::new();
    let mut existing = Vec::new();
    for value in &json.dmcl_patches[1..] {
        let patch: VersionJsonPatch = serde_json::from_value(value.clone())
            .with_context(|| "无法解析已应用的加载器 patch")?;
        match LoaderKind::from_patch_name(&patch.name) {
            Some(kind) => {
                if existing
                    .iter()
                    .any(|(existing_kind, _): &(LoaderKind, VersionJsonPatch)| {
                        *existing_kind == kind
                    })
                {
                    bail!(
                        "实例的 patch 历史里有多个 {} patch，无法判断当前加载器",
                        kind.display_name()
                    );
                }
                existing.push((kind, patch));
            }
            None => unknown.push(patch),
        }
    }

    let existing_has_forge = existing.iter().any(|(kind, _)| *kind == LoaderKind::Forge);
    let has_forge = update
        .loaders
        .iter()
        .any(|request| request.kind == LoaderKind::Forge);

    let mut reused_loaders = Vec::new();
    for (kind, patch) in existing {
        let Some(request) = update.loaders.iter().find(|request| request.kind == kind) else {
            continue;
        };
        let expected = expected_patch_version(kind, &mc_version, &request.version);
        let same_forge_context = kind != LoaderKind::OptiFine || existing_has_forge == has_forge;
        if patch.version == expected && same_forge_context {
            reused_loaders.push((kind, patch));
        }
    }

    let regenerate = update
        .loaders
        .iter()
        .filter(|request| !reused_loaders.iter().any(|(kind, _)| *kind == request.kind))
        .cloned()
        .collect();

    Ok(UpdatePlan {
        base: ResolvedVersion {
            reference: VersionRef::new(root, &update.version_name),
            json: base,
        },
        mc_version,
        has_forge,
        reused: ReusedPatches {
            unknown,
            loaders: reused_loaders,
        },
        regenerate,
    })
}

/// 加载器安装请求里的版本 → 版本 JSON patch 里的 `version`。
///
/// 只有 OptiFine 需要补 Minecraft 前缀；其余加载器直接用请求里的版本。
fn expected_patch_version(kind: LoaderKind, mc_version: &str, version: &str) -> String {
    match kind {
        LoaderKind::OptiFine => format!("{mc_version}_{version}"),
        _ => version.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dmcl_game::GameFilesOptions;
    use dmcl_java::{JavaOption, JreAutoInstall};
    use dmcl_mcmeta::layout::version_dir;
    use dmcl_mcmeta::os::Arch;
    use dmcl_mcmeta::source::{DownloadSource, Mirror};
    use serde_json::json;

    fn root(tag: &str) -> std::path::PathBuf {
        let root =
            std::env::temp_dir().join(format!("dmcl-install-loaders-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(version_dir(&root, "inst")).unwrap();
        root
    }

    fn options(root: &Path) -> Arc<InstallOptions> {
        let java = Arc::new(JavaOption {
            path: std::path::PathBuf::from("/nonexistent/java"),
            major: 17,
            arch: Arch::X86_64,
        });
        Arc::new(InstallOptions::new(
            root,
            GameFilesOptions::new(DownloadSource::new(Mirror::Official)),
            java,
            JreAutoInstall::new(root.join("runtime")),
        ))
    }

    fn write_instance(root: &Path, patches: serde_json::Value) {
        let path = version_json_path(root, "inst");
        std::fs::write(
            &path,
            serde_json::to_string(&json!({
                "id": "inst",
                "dmcl-patches": patches,
            }))
            .unwrap(),
        )
        .unwrap();
    }

    fn request(kind: LoaderKind, version: &str) -> LoaderRequest {
        LoaderRequest {
            kind,
            version: version.to_string(),
        }
    }

    fn kinds(plan: &UpdatePlan) -> (Vec<LoaderKind>, Vec<LoaderKind>) {
        let reused = plan.reused.loaders.iter().map(|(kind, _)| *kind).collect();
        let regenerate = plan.regenerate.iter().map(|request| request.kind).collect();
        (reused, regenerate)
    }

    #[test]
    fn optifine_patch_version_has_the_minecraft_prefix() {
        assert_eq!(
            expected_patch_version(LoaderKind::OptiFine, "1.20.1", "HD_U_I6"),
            "1.20.1_HD_U_I6"
        );
        assert_eq!(
            expected_patch_version(LoaderKind::Forge, "1.20.1", "1.20.1-47.2.0"),
            "1.20.1-47.2.0"
        );
    }

    #[test]
    fn unchanged_loaders_are_reused() {
        let root = root("reuse");
        write_instance(
            &root,
            json!([
                {"id": "1.20.1"},
                {"name": "forge", "version": "1.20.1-47.2.0"},
                {"name": "optifine", "version": "1.20.1_HD_U_I6"}
            ]),
        );
        let update = LoaderUpdate {
            version_name: "inst".to_string(),
            loaders: vec![
                request(LoaderKind::Forge, "1.20.1-47.2.0"),
                request(LoaderKind::OptiFine, "HD_U_I6"),
            ],
        };

        let plan = plan_update(&root, &update).unwrap();
        assert_eq!(
            kinds(&plan),
            (vec![LoaderKind::Forge, LoaderKind::OptiFine], Vec::new())
        );
        assert!(plan.has_forge);

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn changing_one_loader_regenerates_only_that_loader() {
        let root = root("change");
        write_instance(
            &root,
            json!([
                {"id": "1.20.1"},
                {"name": "forge", "version": "1.20.1-47.2.0"},
                {"name": "optifine", "version": "1.20.1_HD_U_I6"}
            ]),
        );
        let update = LoaderUpdate {
            version_name: "inst".to_string(),
            loaders: vec![
                request(LoaderKind::Forge, "1.20.1-47.3.0"),
                request(LoaderKind::OptiFine, "HD_U_I6"),
            ],
        };

        let plan = plan_update(&root, &update).unwrap();
        assert_eq!(
            kinds(&plan),
            (vec![LoaderKind::OptiFine], vec![LoaderKind::Forge])
        );

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn removing_forge_regenerates_optifine_for_the_new_context() {
        let root = root("drop-forge");
        write_instance(
            &root,
            json!([
                {"id": "1.20.1"},
                {"name": "forge", "version": "1.20.1-47.2.0"},
                {"name": "optifine", "version": "1.20.1_HD_U_I6"}
            ]),
        );
        let update = LoaderUpdate {
            version_name: "inst".to_string(),
            loaders: vec![request(LoaderKind::OptiFine, "HD_U_I6")],
        };

        let plan = plan_update(&root, &update).unwrap();
        assert_eq!(kinds(&plan), (Vec::new(), vec![LoaderKind::OptiFine]));
        assert!(!plan.has_forge);

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn custom_patches_are_kept_and_instances_without_patches_are_rejected() {
        let root = root("unknown");
        write_instance(
            &root,
            json!([
                {"id": "1.20.1"},
                {"name": "custom", "version": "1"}
            ]),
        );
        let update = LoaderUpdate {
            version_name: "inst".to_string(),
            loaders: vec![request(LoaderKind::Fabric, "0.15.0")],
        };
        let plan = plan_update(&root, &update).unwrap();
        assert_eq!(plan.reused.unknown.len(), 1);
        assert_eq!(plan.regenerate[0].kind, LoaderKind::Fabric);

        write_instance(&root, json!([]));
        assert!(plan_update(&root, &update).is_err());

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn duplicate_loader_patches_are_rejected() {
        let root = root("duplicate");
        write_instance(
            &root,
            json!([
                {"id": "1.20.1"},
                {"name": "forge", "version": "1.20.1-47.2.0"},
                {"name": "forge", "version": "1.20.1-47.3.0"}
            ]),
        );
        let update = LoaderUpdate {
            version_name: "inst".to_string(),
            loaders: vec![request(LoaderKind::Forge, "1.20.1-47.2.0")],
        };
        assert!(plan_update(&root, &update).is_err());

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn reusing_everything_builds_a_graph_without_installer_steps() {
        let root = root("graph-reuse");
        write_instance(
            &root,
            json!([
                {"id": "1.20.1"},
                {"name": "fabric", "version": "0.15.0"}
            ]),
        );
        let update = LoaderUpdate {
            version_name: "inst".to_string(),
            loaders: vec![request(LoaderKind::Fabric, "0.15.0")],
        };

        let graph = update_loaders_task(options(&root), update).unwrap();
        let names: Vec<&str> = graph
            .steps()
            .iter()
            .map(|step| step.name.as_str())
            .collect();
        assert_eq!(names, ["准备", "完成"]);

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn adding_a_loader_builds_its_install_step() {
        let root = root("graph-add");
        write_instance(&root, json!([{"id": "1.20.1"}]));
        let update = LoaderUpdate {
            version_name: "inst".to_string(),
            loaders: vec![request(LoaderKind::Fabric, "0.15.0")],
        };

        let graph = update_loaders_task(options(&root), update).unwrap();
        let names: Vec<&str> = graph
            .steps()
            .iter()
            .map(|step| step.name.as_str())
            .collect();
        assert!(names.contains(&"安装 Fabric"), "{names:?}");
        assert_eq!(names.last().copied(), Some("完成"));

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[tokio::test]
    async fn reusing_patches_rewrites_the_version_json() {
        let root = root("run-reuse");
        write_instance(
            &root,
            json!([
                {"id": "1.20.1", "type": "release", "libraries": []},
                {
                    "name": "fabric",
                    "version": "0.15.0",
                    "mainClass": "net.fabricmc.loader.impl.launch.knot.KnotClient",
                    "libraries": [{"name": "net.fabricmc:fabric-loader:0.15.0"}]
                }
            ]),
        );
        let update = LoaderUpdate {
            version_name: "inst".to_string(),
            loaders: vec![request(LoaderKind::Fabric, "0.15.0")],
        };

        update_loaders_task(options(&root), update)
            .unwrap()
            .run(|_| {})
            .await
            .unwrap();

        let document: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(version_json_path(&root, "inst")).unwrap(),
        )
        .unwrap();
        assert_eq!(document["id"], "inst");
        assert_eq!(document["jar"], "inst");
        assert_eq!(
            document["mainClass"],
            "net.fabricmc.loader.impl.launch.knot.KnotClient"
        );
        let patches = document["dmcl-patches"].as_array().unwrap();
        assert_eq!(patches.len(), 2);
        assert_eq!(patches[0]["id"], "1.20.1");
        assert_eq!(patches[1]["name"], "fabric");

        std::fs::remove_dir_all(&root).unwrap();
    }
}
