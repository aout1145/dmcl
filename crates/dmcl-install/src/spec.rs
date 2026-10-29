//! 安装计划（"装哪个版本、带哪些加载器"）与两个顶层任务链。

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use dmcl_mcmeta::layout::version_dir;
use dmcl_task::{task_fn, BoxFuture, Task, TaskChain};

use crate::fabric::install_fabric_task;
use crate::files::{client_jar_task, game_files_task};
use crate::forge::install_forge_task;
use crate::neoforge::install_neoforge_task;
use crate::options::InstallOptions;
use crate::vanilla::vanilla_metadata_task;

/// 一种模组加载器。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoaderKind {
    Fabric,
    Forge,
    NeoForge,
}

impl LoaderKind {
    /// 配置文件 / 命令行里用的名字。
    pub fn as_str(self) -> &'static str {
        match self {
            LoaderKind::Fabric => "fabric",
            LoaderKind::Forge => "forge",
            LoaderKind::NeoForge => "neoforge",
        }
    }

    /// 显示用的名字。
    pub fn display_name(self) -> &'static str {
        match self {
            LoaderKind::Fabric => "Fabric",
            LoaderKind::Forge => "Forge",
            LoaderKind::NeoForge => "NeoForge",
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

impl LoaderRequest {
    /// 变成对应加载器的安装任务链。
    pub fn task(
        &self,
        options: Arc<InstallOptions>,
        version_name: String,
        mc_version: String,
    ) -> TaskChain<(), ()> {
        let name = version_name.clone();
        match self.kind {
            LoaderKind::Fabric => {
                install_fabric_task(options, name, mc_version, self.version.clone())
            }
            LoaderKind::Forge => install_forge_task(options, name, self.version.clone()),
            LoaderKind::NeoForge => install_neoforge_task(options, name, self.version.clone()),
        }
    }
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

/// 占用版本目录：已存在就直接报错，不存在则创建它。
///
/// 删除清理由安装链在**这一步成功之后**通过 `set_cleanup` 登记；因此版本已存在时
/// 清理不会生效，也就不会误删用户已有的版本。
fn create_version_dir(root_path: PathBuf, version_name: String) -> impl Task<(), ()> + 'static {
    task_fn(move |_ctx| {
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
                Ok(()) => Ok(()),
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
fn remove_version_dir_cleanup(
    root_path: PathBuf,
    version_name: String,
) -> impl Fn() -> BoxFuture<'static, ()> + Send + Sync + 'static {
    move || {
        let path = version_dir(&root_path, &version_name);
        Box::pin(async move {
            match tokio::fs::remove_dir_all(&path).await {
                Ok(()) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => {
                    eprintln!("警告：无法删除未完成的安装 {}：{err}", path.display());
                }
            }
        })
    }
}

/// 完整的安装链：原版元数据 → 客户端 jar → 各加载器 → 游戏文件。
///
/// 顺序不能变：客户端 jar 必须先于加载器就位，因为 Forge / NeoForge 的
/// processor 要读它来生成 patched client（`{MINECRAFT_JAR}`）。
///
/// 安装是「版本级原子」的：第一步就占用 `versions/<version_name>`，目录已存在时直接
/// 报错（不会碰已有版本）；只有这一步成功后才会走到 `set_cleanup` 登记删除动作，
/// 之后任何一步失败或被取消，都会删掉本次新建的版本目录，不留下半装的版本。
pub fn install_task(options: Arc<InstallOptions>, spec: InstallSpec) -> TaskChain<(), ()> {
    let mut chain = TaskChain::new(format!("安装 {}", spec.version_name))
        .then(
            create_version_dir(options.root_path.clone(), spec.version_name.clone()),
            "创建版本目录",
        )
        .set_cleanup(remove_version_dir_cleanup(
            options.root_path.clone(),
            spec.version_name.clone(),
        ))
        .then(
            vanilla_metadata_task(
                options.clone(),
                spec.version_id.clone(),
                spec.version_name.clone(),
            ),
            "获取版本 json",
        )
        .then(
            client_jar_task(options.clone(), spec.version_name.clone()),
            "下载客户端 jar",
        );

    for request in &spec.loaders {
        chain = chain.then(
            request.task(
                options.clone(),
                spec.version_name.clone(),
                spec.version_id.clone(),
            ),
            format!("安装 {}", request.kind.display_name()),
        );
    }

    chain.then(
        game_files_task(options, spec.version_name.clone()),
        "下载游戏文件",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use dmcl_mcmeta::source::{DownloadSource, Mirror};

    #[test]
    fn specs_collect_loader_requests_in_order() {
        let spec = InstallSpec::new("1.20.1", "1.20.1")
            .with_fabric("0.15.0")
            .with_loader(LoaderKind::Forge, "47.2.0");
        assert_eq!(spec.loaders.len(), 2);
        assert_eq!(spec.loaders[0].kind, LoaderKind::Fabric);
        assert_eq!(spec.loaders[0].version, "0.15.0");
        assert_eq!(spec.loaders[1].version, "47.2.0");
        assert_eq!(LoaderKind::NeoForge.to_string(), "neoforge");
    }

    #[tokio::test]
    async fn a_failed_install_removes_the_version_directory() {
        let root = std::env::temp_dir().join(format!("dmcl-install-abort-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&root).await;
        let dir = version_dir(&root, "1.0");

        let chain = TaskChain::new("安装 1.0")
            .then(
                create_version_dir(root.clone(), "1.0".into()),
                "检查版本是否已存在",
            )
            .set_cleanup(remove_version_dir_cleanup(root.clone(), "1.0".into()))
            .then(
                dmcl_task::task_fn(|_ctx| async move { Err::<(), _>(anyhow::anyhow!("故意失败")) }),
                "会失败的一步",
            );
        chain.run(()).await.unwrap_err();

        assert!(!dir.exists(), "失败的安装不应留下版本目录");
        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    #[tokio::test]
    async fn a_successful_install_keeps_the_version_directory() {
        let root = std::env::temp_dir().join(format!("dmcl-install-ok-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&root).await;
        let dir = version_dir(&root, "1.0");

        let chain = TaskChain::new("安装 1.0")
            .then(
                create_version_dir(root.clone(), "1.0".into()),
                "检查版本是否已存在",
            )
            .set_cleanup(remove_version_dir_cleanup(root.clone(), "1.0".into()))
            .then(
                dmcl_task::task_fn(|_ctx| async move { Ok(()) }),
                "成功的一步",
            );
        chain.run(()).await.unwrap();

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

        let options = Arc::new(InstallOptions::new(
            &root,
            DownloadSource::new(Mirror::Official),
        ));
        let err = install_task(options, InstallSpec::new("1.0", "1.0"))
            .run(())
            .await
            .unwrap_err();

        assert!(format!("{err:#}").contains("已存在"), "{err:#}");
        assert!(dir.exists(), "已存在的版本目录不能被删除清理误删");
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
    }
}
