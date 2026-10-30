//! `install` —— 安装原版，可顺带装模组加载器。

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;

use dmcl_install::{default_version_name, install_task, InstallSpec, LoaderKind};
use dmcl_mcmeta::source::DownloadSource;

use crate::config::Config;

use super::handle_event;
use crate::cli::progress::ProgressPrinter;

/// `install` 命令的参数。
pub(crate) struct InstallArgs {
    /// Minecraft 版本号。
    pub version_id: String,
    /// 本地版本目录名；省略时自动取名。
    pub version_name: Option<String>,
    pub forge: bool,
    pub neoforge: bool,
    pub fabric: bool,
    pub forge_version: Option<String>,
    pub neoforge_version: Option<String>,
    pub fabric_version: Option<String>,
    pub optifine: bool,
    pub optifine_version: Option<String>,
}

impl InstallArgs {
    /// 请求了哪些加载器（固定顺序：forge → neoforge → fabric）。
    fn requested_loaders(&self) -> Vec<(LoaderKind, Option<&str>)> {
        let mut loaders = Vec::new();
        if self.forge {
            loaders.push((LoaderKind::Forge, self.forge_version.as_deref()));
        }
        if self.neoforge {
            loaders.push((LoaderKind::NeoForge, self.neoforge_version.as_deref()));
        }
        if self.fabric {
            loaders.push((LoaderKind::Fabric, self.fabric_version.as_deref()));
        }
        if self.optifine {
            loaders.push((LoaderKind::OptiFine, self.optifine_version.as_deref()));
        }
        loaders
    }
}

/// `dmcl install <mc-version-id> [version-name] [--forge …]`。
pub(crate) async fn run(args: InstallArgs, config: &Config, root_path: &Path) -> Result<()> {
    let source = config.download_source();

    // `dmcl-install` 现在强制要求具体版本：没给就现场查最新。
    let mut loaders: Vec<(LoaderKind, String)> = Vec::new();
    for (kind, requested) in args.requested_loaders() {
        let version = match requested {
            Some(version) => version.to_string(),
            None => latest_loader(&source, kind, &args.version_id).await?,
        };
        loaders.push((kind, version));
    }

    let version_name = match &args.version_name {
        Some(name) => name.clone(),
        None => {
            let with_versions: Vec<(LoaderKind, Option<&str>)> = loaders
                .iter()
                .map(|(kind, version)| (*kind, Some(version.as_str())))
                .collect();
            default_version_name(&args.version_id, &with_versions)
        }
    };

    for (kind, version) in &loaders {
        println!("使用 {} {version}", kind.display_name());
    }
    println!("安装为 {version_name}");

    let mut spec = InstallSpec::new(args.version_id.clone(), version_name);
    for (kind, version) in &loaders {
        spec = spec.with_loader(*kind, version.clone());
    }

    let launch = config.launch.clone();
    let options = Arc::new(config.install_options(source, &launch, root_path)?);
    let mut printer = ProgressPrinter::new();
    let result = install_task(options, spec)
        .run_with((), |event| handle_event(&mut printer, event))
        .await;
    printer.finish();
    result
}

/// 现场查加载器最新版本（install 没给版本时用）。
///
/// 三个 `fetch_*` 都按版本**从新到旧**返回，因此「最新」＝列表第一条；Fabric 额外
/// 优先稳定版（找不到稳定版才退回最新一条）。
async fn latest_loader(
    source: &DownloadSource,
    kind: LoaderKind,
    mc_version: &str,
) -> Result<String> {
    let version = match kind {
        LoaderKind::Fabric => {
            let list = source.fetch_fabric_loaders(mc_version).await?;
            list.iter()
                .find(|entry| entry.loader.is_stable())
                .or_else(|| list.first())
                .map(|entry| entry.loader.version.clone())
                .unwrap_or_default()
        }
        LoaderKind::Forge => source
            .fetch_forge_versions(mc_version)
            .await?
            .first()
            .map(dmcl_mcmeta::forge::ForgeVersionEntry::full_name)
            .unwrap_or_default(),
        LoaderKind::NeoForge => source
            .fetch_neoforge_versions(mc_version)
            .await?
            .first()
            .map(|entry| entry.version.clone())
            .unwrap_or_default(),
        LoaderKind::OptiFine => source.latest_optifine_version(mc_version).await?,
    };
    Ok(version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requested_loaders_follow_a_fixed_order() {
        let args = InstallArgs {
            version_id: "1.20.1".into(),
            version_name: None,
            forge: true,
            neoforge: false,
            fabric: true,
            forge_version: Some("47.2.0".into()),
            neoforge_version: None,
            fabric_version: None,
            optifine: true,
            optifine_version: Some("HD_U_H8".into()),
        };
        let loaders = args.requested_loaders();
        assert_eq!(loaders[0].0, LoaderKind::Forge);
        assert_eq!(loaders[1].0, LoaderKind::Fabric);
        assert_eq!(loaders[2].0, LoaderKind::OptiFine);
    }
}
