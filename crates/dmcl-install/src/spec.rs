//! 安装计划（"装哪个版本、带哪些加载器"）与两个顶层任务链。

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use dmcl_download::DownloadContext;
use dmcl_game::{ensure_client_jar, ensure_files, resolve_version, ResolvedVersion, VersionRef};
use dmcl_mcmeta::layout::version_dir;
use dmcl_task::{task_fn, Task, TaskContext, TaskGraph, TaskGraphBuilder};

use crate::fabric::install_fabric;
use crate::forge::install_forge;
use crate::neoforge::install_neoforge;
use crate::optifine::install_optifine;
use crate::options::InstallOptions;
use crate::vanilla::download_vanilla;

/// 一种模组加载器，或叠加在原版 / Forge 上的 OptiFine。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoaderKind {
    Fabric,
    Forge,
    NeoForge,
    OptiFine,
}

impl LoaderKind {
    /// 配置文件 / 命令行里用的名字。
    pub fn as_str(self) -> &'static str {
        match self {
            LoaderKind::Fabric => "fabric",
            LoaderKind::Forge => "forge",
            LoaderKind::NeoForge => "neoforge",
            LoaderKind::OptiFine => "optifine",
        }
    }

    /// 显示用的名字。
    pub fn display_name(self) -> &'static str {
        match self {
            LoaderKind::Fabric => "Fabric",
            LoaderKind::Forge => "Forge",
            LoaderKind::NeoForge => "NeoForge",
            LoaderKind::OptiFine => "OptiFine",
        }
    }
}

impl std::fmt::Display for LoaderKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 一个加载器安装请求。
#[derive(Debug, Clone)]
pub struct LoaderRequest {
    pub kind: LoaderKind,
    /// 要安装的加载器版本（调用者负责确定具体版本，`dmcl-install` 不查最新）。
    pub version: String,
}

/// 一次安装的完整计划。
#[derive(Debug, Clone)]
pub struct InstallSpec {
    /// 要装的 Minecraft 版本号（版本列表里的 id）。
    pub version_id: String,
    /// 本地目录名（`versions/<version_name>`），通常与 `version_id` 相同。
    pub version_name: String,
    /// 额外要装的加载器，按顺序执行。
    pub loaders: Vec<LoaderRequest>,
}

impl InstallSpec {
    pub fn new(version_id: impl Into<String>, version_name: impl Into<String>) -> Self {
        Self {
            version_id: version_id.into(),
            version_name: version_name.into(),
            loaders: Vec::new(),
        }
    }

    /// 追加一个加载器请求。
    pub fn with_loader(mut self, kind: LoaderKind, version: impl Into<String>) -> Self {
        self.loaders.push(LoaderRequest {
            kind,
            version: version.into(),
        });
        self
    }

    pub fn with_fabric(self, version: impl Into<String>) -> Self {
        self.with_loader(LoaderKind::Fabric, version)
    }

    pub fn with_forge(self, version: impl Into<String>) -> Self {
        self.with_loader(LoaderKind::Forge, version)
    }

    pub fn with_neoforge(self, version: impl Into<String>) -> Self {
        self.with_loader(LoaderKind::NeoForge, version)
    }

    pub fn with_optifine(self, version: impl Into<String>) -> Self {
        self.with_loader(LoaderKind::OptiFine, version)
    }
}

/// 默认的本地版本目录名：`mcversion-[loadername[-loaderversion]]*n`。
///
/// 加载器版本未知（`None`）时只写加载器名（`1.20.1-forge`），不硬凑版本号。
/// CLI 与 GUI 共用（新实例页的自动命名）。
pub fn default_version_name(mc_version: &str, loaders: &[(LoaderKind, Option<&str>)]) -> String {
    let mut name = mc_version.to_string();
    for (kind, version) in loaders {
        name.push('-');
        name.push_str(kind.as_str());
        if let Some(version) = version {
            name.push('-');
            name.push_str(version);
        }
    }
    name
}

/// 一次安装的产出。
#[derive(Debug, Clone)]
pub struct InstallOutcome {
    pub version: VersionRef,
}

/// 占用版本目录：已存在就直接报错，不存在则创建它。
///
/// 删除动作在**这一步成功之后**才通过 `on_rollback` 登记；因此版本已存在时回滚不会
/// 生效，也就不会误删用户已有的版本。
fn create_version_dir(
    root_path: PathBuf,
    version_name: String,
) -> impl Task<(), VersionRef> + 'static {
    task_fn(move |_: (), ctx| {
        let root_path = root_path.clone();
        let version_name = version_name.clone();
        async move {
            let dir = version_dir(&root_path, &version_name);
            if let Some(parent) = dir.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .with_context(|| format!("无法创建 {}", parent.display()))?;
            }
            match tokio::fs::create_dir(&dir).await {
                Ok(()) => {
                    let cleanup_root = root_path.clone();
                    let cleanup_name = version_name.clone();
                    ctx.on_rollback(move |ctx| {
                        let root_path = cleanup_root.clone();
                        let version_name = cleanup_name.clone();
                        async move { remove_version_dir(&root_path, &version_name, &ctx).await }
                    });
                    Ok(VersionRef::new(root_path, version_name))
                }
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                    anyhow::bail!("版本「{version_name}」已存在：{}", dir.display())
                }
                Err(err) => Err(err).with_context(|| format!("无法创建版本目录 {}", dir.display())),
            }
        }
    })
}

/// 安装中止（失败或被取消）时删除写到一半的版本目录。
///
/// 共享的 `libraries/` / `assets/` 是内容校验过的缓存，不跟着回滚；只要版本目录不在，
/// 这个版本就不会被当成已安装，下次安装会重新补齐。
async fn remove_version_dir(root_path: &std::path::Path, version_name: &str, ctx: &TaskContext) {
    let path = version_dir(root_path, version_name);
    match tokio::fs::remove_dir_all(&path).await {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => {
            ctx.log(format!(
                "警告：无法删除未完成的安装 {}：{err}",
                path.display()
            ));
        }
    }
}

/// 获取原版元数据并落盘，然后读回解析好的版本。
fn vanilla_metadata_step(
    options: Arc<InstallOptions>,
    version_id: String,
) -> impl Task<Arc<VersionRef>, ResolvedVersion> + 'static {
    task_fn(move |reference: Arc<VersionRef>, ctx: TaskContext| {
        let options = options.clone();
        let version_id = version_id.clone();
        async move {
            let reference = (*reference).clone();
            ctx.log(format!("查询版本列表并保存 {}.json", reference.name));
            download_vanilla(&options, &version_id, &reference.name).await?;
            ctx.log(format!("{}.json 已写入", reference.name));
            resolve_version(&reference).await
        }
    })
}

/// 确保客户端 jar 就位（加载器 processor 之前需要）。
fn ensure_client_jar_step(
    options: Arc<InstallOptions>,
    downloads: Arc<DownloadContext>,
) -> impl Task<Arc<ResolvedVersion>, ResolvedVersion> + 'static {
    task_fn(move |version: Arc<ResolvedVersion>, ctx: TaskContext| {
        let options = options.clone();
        let downloads = downloads.clone();
        async move {
            ensure_client_jar(&version, &options.game, &downloads, &ctx).await?;
            Ok((*version).clone())
        }
    })
}

/// 补齐版本需要的全部文件，并产出安装结果。
fn materialize_step(
    options: Arc<InstallOptions>,
    downloads: Arc<DownloadContext>,
) -> impl Task<Arc<ResolvedVersion>, InstallOutcome> + 'static {
    task_fn(move |version: Arc<ResolvedVersion>, ctx: TaskContext| {
        let options = options.clone();
        let downloads = downloads.clone();
        async move {
            ensure_files(&version, &options.game, &downloads, &ctx).await?;
            Ok(InstallOutcome {
                version: version.reference.clone(),
            })
        }
    })
}

/// 完整的安装任务图：原版元数据 → 客户端 jar → 各加载器 → 游戏文件。
///
/// 顺序不能变：客户端 jar 必须先于加载器就位，因为 Forge / NeoForge 的
/// processor 要读它来生成 patched client（`{MINECRAFT_JAR}`）。
///
/// 安装是「版本级原子」的：先占用 `versions/<version_name>`，目录已存在时直接报错
/// （不会碰已有版本）；只有占用成功后才会登记回滚动作，之后任何一步失败或被取消，都会
/// 删掉本次新建的版本目录，不留下半装的版本。
pub fn install_task(options: Arc<InstallOptions>, spec: InstallSpec) -> TaskGraph<InstallOutcome> {
    // 加载器的先后顺序由调用方决定；这里不额外调整顺序或检查组合兼容性。
    let mut builder = TaskGraphBuilder::new(format!("安装 {}", spec.version_name));
    let downloads = options.game.download_context(builder.progress());

    let prepare = builder.step("准备");
    let created = builder.add(
        prepare,
        "创建版本目录",
        (),
        create_version_dir(options.root_path.clone(), spec.version_name.clone()),
    );
    let resolved = builder.add(
        prepare,
        "获取版本 json",
        created,
        vanilla_metadata_step(options.clone(), spec.version_id.clone()),
    );
    let mut previous = builder.add(
        prepare,
        "下载客户端 jar",
        resolved,
        ensure_client_jar_step(options.clone(), downloads.clone()),
    );

    let has_forge = spec
        .loaders
        .iter()
        .any(|request| request.kind == LoaderKind::Forge);
    for request in &spec.loaders {
        let step = builder.step(format!("安装 {}", request.kind.display_name()));
        let options = options.clone();
        let downloads = downloads.clone();
        let request = request.clone();
        let version_name = spec.version_name.clone();
        let mc_version = spec.version_id.clone();
        previous = builder.add(
            step,
            format!("安装 {}", request.kind.display_name()),
            previous.clone(),
            task_fn(move |version: Arc<ResolvedVersion>, ctx| {
                let options = options.clone();
                let downloads = downloads.clone();
                let request = request.clone();
                let version_name = version_name.clone();
                let mc_version = mc_version.clone();
                async move {
                    match request.kind {
                        LoaderKind::Fabric => {
                            install_fabric(&options, &version_name, &mc_version, &request.version)
                                .await?
                        }
                        LoaderKind::Forge => {
                            install_forge(
                                &options,
                                &downloads,
                                &version_name,
                                &request.version,
                                &ctx,
                            )
                            .await?
                        }
                        LoaderKind::NeoForge => {
                            install_neoforge(
                                &options,
                                &downloads,
                                &version_name,
                                &request.version,
                                &ctx,
                            )
                            .await?
                        }
                        LoaderKind::OptiFine => {
                            install_optifine(
                                &options,
                                &downloads,
                                &version_name,
                                &mc_version,
                                &request.version,
                                has_forge,
                                &ctx,
                            )
                            .await?
                        }
                    }
                    resolve_version(&version.reference).await
                }
            }),
        );
    }

    let download_step = builder.step("下载游戏文件");
    let sink = builder.add(
        download_step,
        "下载游戏文件",
        previous,
        materialize_step(options, downloads),
    );
    builder.build(sink).expect("安装图的最终节点由构造保证")
}

#[cfg(test)]
mod tests {
    use super::*;
    use dmcl_game::GameFilesOptions;
    use dmcl_java::{JavaOption, JreAutoInstall};
    use dmcl_mcmeta::os::Arch;
    use dmcl_mcmeta::source::{DownloadSource, Mirror};

    fn options(root: impl Into<PathBuf>) -> Arc<InstallOptions> {
        let root = root.into();
        let java = Arc::new(JavaOption {
            path: PathBuf::from("/nonexistent/java"),
            major: 17,
            arch: Arch::X86_64,
        });
        let jre = JreAutoInstall::new(root.join("runtime"));
        Arc::new(InstallOptions::new(
            root,
            GameFilesOptions::new(DownloadSource::new(Mirror::Official)),
            java,
            jre,
        ))
    }

    #[test]
    fn specs_collect_loader_requests_in_order() {
        let spec = InstallSpec::new("1.20.1", "1.20.1")
            .with_fabric("0.15.0")
            .with_loader(LoaderKind::Forge, "47.2.0");
        assert_eq!(spec.loaders.len(), 2);
        assert_eq!(spec.loaders[0].kind, LoaderKind::Fabric);
        assert_eq!(spec.loaders[0].version, "0.15.0");
        assert_eq!(spec.loaders[1].version, "47.2.0");
    }

    #[tokio::test]
    async fn a_failed_install_removes_the_version_directory() {
        let root = std::env::temp_dir().join(format!("dmcl-install-abort-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&root).await;
        let dir = version_dir(&root, "1.0");

        let mut builder = TaskGraphBuilder::new("安装 1.0");
        let step = builder.step("准备");
        let created = builder.add(
            step,
            "创建版本目录",
            (),
            create_version_dir(root.clone(), "1.0".into()),
        );
        let sink = builder.add(
            step,
            "会失败的一步",
            created,
            task_fn(|_version: Arc<VersionRef>, _ctx| async move {
                Err::<(), _>(anyhow::anyhow!("故意失败"))
            }),
        );
        builder.build(sink).unwrap().run(|_| {}).await.unwrap_err();

        assert!(!dir.exists(), "失败的安装不应留下版本目录");
        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    #[tokio::test]
    async fn a_successful_install_keeps_the_version_directory() {
        let root = std::env::temp_dir().join(format!("dmcl-install-ok-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&root).await;
        let dir = version_dir(&root, "1.0");

        let mut builder = TaskGraphBuilder::new("安装 1.0");
        let step = builder.step("准备");
        let created = builder.add(
            step,
            "创建版本目录",
            (),
            create_version_dir(root.clone(), "1.0".into()),
        );
        let sink = builder.add(
            step,
            "成功的一步",
            created,
            task_fn(|_version: Arc<VersionRef>, _ctx| async move { Ok(()) }),
        );
        builder.build(sink).unwrap().run(|_| {}).await.unwrap();

        assert!(dir.exists(), "成功的安装必须保留版本目录");
        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    #[tokio::test]
    async fn an_existing_version_is_rejected_without_deleting_it() {
        let root = std::env::temp_dir().join(format!("dmcl-install-exists-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&root).await;
        let dir = version_dir(&root, "1.0");
        tokio::fs::create_dir_all(&dir).await.unwrap();
        tokio::fs::write(dir.join("1.0.json"), "{}").await.unwrap();

        let options = options(root.clone());
        let err = install_task(options, InstallSpec::new("1.0", "1.0"))
            .run(|_| {})
            .await
            .unwrap_err();

        assert!(format!("{err:#}").contains("已存在"), "{err:#}");
        assert!(dir.exists(), "已存在的版本目录不能被回滚误删");
        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    #[test]
    fn default_name_follows_mcversion_and_loaders() {
        assert_eq!(default_version_name("1.20.1", &[]), "1.20.1");
        assert_eq!(
            default_version_name("1.20.1", &[(LoaderKind::Forge, Some("47.2.0"))]),
            "1.20.1-forge-47.2.0"
        );
        // 没给版本的加载器只写名字
        assert_eq!(
            default_version_name("1.20.1", &[(LoaderKind::Forge, None)]),
            "1.20.1-forge"
        );
        assert_eq!(
            default_version_name(
                "1.20.1",
                &[
                    (LoaderKind::Forge, Some("47.2.0")),
                    (LoaderKind::Fabric, Some("0.15.0")),
                ]
            ),
            "1.20.1-forge-47.2.0-fabric-0.15.0"
        );
        assert_eq!(
            default_version_name("1.19", &[(LoaderKind::OptiFine, Some("HD_U_H8"))]),
            "1.19-optifine-HD_U_H8"
        );
    }

    #[test]
    fn loaders_keep_the_callers_order() {
        let options = options("/tmp/.minecraft");
        let spec = InstallSpec::new("1.19", "1.19")
            .with_optifine("HD_U_H8")
            .with_forge("1.19-47.3.0");
        let steps: Vec<String> = spec
            .loaders
            .iter()
            .map(|request| format!("安装 {}", request.kind.display_name()))
            .collect();
        let graph = install_task(options, spec);
        let names: Vec<_> = graph.steps().iter().map(|step| step.name.clone()).collect();
        let positions: Vec<_> = steps
            .iter()
            .map(|step| {
                names
                    .iter()
                    .position(|name| name == step)
                    .unwrap_or_else(|| panic!("缺少步骤「{step}」：{names:?}"))
            })
            .collect();
        assert!(
            positions.windows(2).all(|pair| pair[0] < pair[1]),
            "{names:?}"
        );
    }
}
