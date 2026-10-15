//! `install` —— 安装原版，可顺带装模组加载器。

use std::sync::Arc;

use anyhow::Result;

use dmcl_install::{install_task, InstallSpec, LoaderKind};
use dmcl_mcmeta::fabric::latest_fabric_loader;
use dmcl_mcmeta::forge::latest_forge_version;
use dmcl_mcmeta::neoforge::latest_neoforge_version;
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
        loaders
    }
}

/// `dmcl install <mc-version-id> [version-name] [--forge …]`。
pub(crate) async fn run(args: InstallArgs, config: &Config) -> Result<()> {
    let source = config.download_source();

    // `dmcl-install` 现在强制要求具体版本：没给就现场查最新。
    let mut loaders: Vec<(LoaderKind, String)> = Vec::new();
    for (kind, requested) in args.requested_loaders() {
        let version = match requested {
            Some(version) => version.to_string(),
            None => resolve_latest(&source, kind, &args.version_id).await?,
        };
        loaders.push((kind, version));
    }

    let version_name = args
        .version_name
        .clone()
        .unwrap_or_else(|| default_version_name(&args.version_id, &loaders));

    for (kind, version) in &loaders {
        println!("使用 {} {version}", kind.display_name());
    }
    println!("安装为 {version_name}");

    let mut spec = InstallSpec::new(args.version_id.clone(), version_name);
    for (kind, version) in &loaders {
        spec = spec.with_loader(*kind, version.clone());
    }

    let options = Arc::new(config.install_options(source));
    let mut printer = ProgressPrinter::new();
    let result = install_task(options, spec)
        .run_with((), |event| handle_event(&mut printer, event))
        .await;
    printer.finish();
    result
}

/// 查某个加载器在某个 mc 版本下的最新版。
async fn resolve_latest(
    source: &DownloadSource,
    kind: LoaderKind,
    mc_version: &str,
) -> Result<String> {
    match kind {
        LoaderKind::Fabric => latest_fabric_loader(source, mc_version).await,
        LoaderKind::Forge => latest_forge_version(source, mc_version).await,
        LoaderKind::NeoForge => latest_neoforge_version(source, mc_version).await,
    }
}

/// 默认的本地版本目录名：`mcversion-[loadername-loaderversion]*n`。
fn default_version_name(mc_version: &str, loaders: &[(LoaderKind, String)]) -> String {
    let mut name = mc_version.to_string();
    for (kind, version) in loaders {
        name.push_str(&format!("-{}-{version}", kind.as_str()));
    }
    name
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_name_follows_mcversion_and_loaders() {
        assert_eq!(default_version_name("1.20.1", &[]), "1.20.1");
        assert_eq!(
            default_version_name("1.20.1", &[(LoaderKind::Forge, "47.2.0".to_string())]),
            "1.20.1-forge-47.2.0"
        );
        assert_eq!(
            default_version_name(
                "1.20.1",
                &[
                    (LoaderKind::Forge, "47.2.0".to_string()),
                    (LoaderKind::Fabric, "0.15.0".to_string()),
                ]
            ),
            "1.20.1-forge-47.2.0-fabric-0.15.0"
        );
    }

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
        };
        let loaders = args.requested_loaders();
        assert_eq!(loaders[0].0, LoaderKind::Forge);
        assert_eq!(loaders[1].0, LoaderKind::Fabric);
    }
}
