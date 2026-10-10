//! 安装计划（"装哪个版本、带哪些加载器"）与两个顶层任务链。

use std::sync::Arc;

use dmcl_task::TaskChain;

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
    /// 想要的加载器版本；`None` 表示"自动取最新"。
    pub version: Option<String>,
}

impl LoaderRequest {
    /// 装最新版。
    pub fn latest(kind: LoaderKind) -> Self {
        Self {
            kind,
            version: None,
        }
    }

    /// 装指定版本。
    pub fn version(kind: LoaderKind, version: impl Into<String>) -> Self {
        Self {
            kind,
            version: Some(version.into()),
        }
    }

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
            LoaderKind::Forge => {
                install_forge_task(options, name, mc_version, self.version.clone())
            }
            LoaderKind::NeoForge => {
                install_neoforge_task(options, name, mc_version, self.version.clone())
            }
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
    pub fn with_loader(mut self, kind: LoaderKind, version: Option<String>) -> Self {
        self.loaders.push(LoaderRequest { kind, version });
        self
    }

    /// 追加"装最新版"的加载器请求。
    pub fn with_latest_loader(self, kind: LoaderKind) -> Self {
        self.with_loader(kind, None)
    }

    pub fn with_fabric(self, version: Option<String>) -> Self {
        self.with_loader(LoaderKind::Fabric, version)
    }

    pub fn with_forge(self, version: Option<String>) -> Self {
        self.with_loader(LoaderKind::Forge, version)
    }

    pub fn with_neoforge(self, version: Option<String>) -> Self {
        self.with_loader(LoaderKind::NeoForge, version)
    }
}

/// 完整的安装链：原版元数据 → 客户端 jar → 各加载器 → 游戏文件。
///
/// 顺序不能变：客户端 jar 必须先于加载器就位，因为 Forge / NeoForge 的
/// processor 要读它来生成 patched client（`{MINECRAFT_JAR}`）。
pub fn install_task(options: Arc<InstallOptions>, spec: InstallSpec) -> TaskChain<(), ()> {
    let mut chain = TaskChain::new(format!("安装 {}", spec.version_name))
        .then(
            vanilla_metadata_task(
                options.clone(),
                spec.version_id.clone(),
                spec.version_name.clone(),
            ),
            "获取原版元数据",
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

/// 在**已有**的版本上装一个加载器，然后补齐文件。
///
/// 对应原来的 `install-forge` / `install-neoforge` 命令。客户端 jar 同样先
/// 确保就位（如果版本已经装过，这一步会被"已是最新"跳过）。
pub fn install_loader_task(
    options: Arc<InstallOptions>,
    version_name: String,
    mc_version: String,
    request: LoaderRequest,
) -> TaskChain<(), ()> {
    TaskChain::new(format!("安装 {}", request.kind.display_name()))
        .then(
            client_jar_task(options.clone(), version_name.clone()),
            "下载客户端 jar",
        )
        .then(
            request.task(options.clone(), version_name.clone(), mc_version),
            format!("安装 {}", request.kind.display_name()),
        )
        .then(game_files_task(options, version_name), "下载游戏文件")
}

#[cfg(test)]
mod tests {
    use super::*;
    use dmcl_mcmeta::{DownloadSource, Mirror};

    fn options() -> Arc<InstallOptions> {
        Arc::new(InstallOptions::new(
            "/tmp/mc",
            DownloadSource::new(Mirror::Official),
        ))
    }

    #[test]
    fn specs_collect_loader_requests_in_order() {
        let spec = InstallSpec::new("1.20.1", "1.20.1")
            .with_latest_loader(LoaderKind::Fabric)
            .with_loader(LoaderKind::Forge, Some("47.2.0".to_string()));
        assert_eq!(spec.loaders.len(), 2);
        assert_eq!(spec.loaders[0].kind, LoaderKind::Fabric);
        assert!(spec.loaders[0].version.is_none());
        assert_eq!(spec.loaders[1].version.as_deref(), Some("47.2.0"));
        assert_eq!(LoaderKind::NeoForge.to_string(), "neoforge");
    }

    #[test]
    fn the_install_chain_has_one_step_per_phase() {
        let spec = InstallSpec::new("1.20.1", "1.20.1").with_fabric(None);
        let chain = install_task(options(), spec);
        let steps: Vec<&str> = chain.step_names().collect();
        // 客户端 jar 必须先于加载器：Forge/NeoForge 的 processor 要读它
        assert_eq!(
            steps,
            vec![
                "获取原版元数据",
                "下载客户端 jar",
                "安装 Fabric",
                "下载游戏文件"
            ]
        );
        assert_eq!(chain.name(), "安装 1.20.1");
    }

    #[test]
    fn the_loader_chain_installs_then_downloads() {
        let chain = install_loader_task(
            options(),
            "1.20.1".to_string(),
            "1.20.1".to_string(),
            LoaderRequest::version(LoaderKind::Forge, "47.2.0"),
        );
        let steps: Vec<&str> = chain.step_names().collect();
        assert_eq!(steps, vec!["下载客户端 jar", "安装 Forge", "下载游戏文件"]);
    }
}
