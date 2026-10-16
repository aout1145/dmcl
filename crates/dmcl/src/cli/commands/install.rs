//! `install` —— 安装原版，可顺带装模组加载器。

use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::Result;

use dmcl_install::{install_task, InstallSpec, LoaderKind};
use dmcl_mcmeta::source::DownloadSource;
use dmcl_task::BoxFuture;

use crate::cli::prompt::ArgDefaults;
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
            None => source.latest(kind, &args.version_id).await?,
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

    let options = Arc::new(config.install_options(source));
    let mut printer = ProgressPrinter::new();
    let result = install_task(options, spec)
        .run_with((), |event| handle_event(&mut printer, event))
        .await;
    printer.finish();
    result
}

// ---------------------------------------------------------------------------
// 加载器：参数映射、命名规则、版本解析
// ---------------------------------------------------------------------------

/// 这个参数 id 是不是某个加载器的**版本**参数（`forge_version` → `Forge`）。
pub(crate) fn loader_version_arg(id: &str) -> Option<LoaderKind> {
    match id {
        "forge_version" => Some(LoaderKind::Forge),
        "neoforge_version" => Some(LoaderKind::NeoForge),
        "fabric_version" => Some(LoaderKind::Fabric),
        _ => None,
    }
}

/// 这个参数 id 是不是某个加载器的**开关**（`forge` → `Forge`）。
pub(crate) fn loader_flag_arg(id: &str) -> Option<LoaderKind> {
    match id {
        "forge" => Some(LoaderKind::Forge),
        "neoforge" => Some(LoaderKind::NeoForge),
        "fabric" => Some(LoaderKind::Fabric),
        _ => None,
    }
}

/// 默认的本地版本目录名：`mcversion-[loadername[-loaderversion]]*n`。
///
/// 加载器版本未知（`None`）时只写加载器名（`1.20.1-forge`），不硬凑版本号。
pub(crate) fn default_version_name(
    mc_version: &str,
    loaders: &[(LoaderKind, Option<&str>)],
) -> String {
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

/// 从 prompt 收集到的答案里拼默认版本名（`version-name` 的默认值）。
///
/// `enabled_flags` 是已启用的开关 id，其中属于加载器的会写进名字；没有 mc 版本时
/// 返回 `None`。prompt 因此不需要知道 `LoaderKind`。
pub(crate) fn default_version_name_for_prompt(
    answers: &BTreeMap<String, String>,
    enabled_flags: &[String],
) -> Option<String> {
    let mc_version = answers.get("version_id")?;
    let loaders: Vec<(LoaderKind, Option<&str>)> = enabled_flags
        .iter()
        .filter_map(|flag| loader_flag_arg(flag))
        .map(|kind| {
            let version = answers
                .iter()
                .find(|(id, _)| loader_version_arg(id) == Some(kind))
                .map(|(_, version)| version.as_str());
            (kind, version)
        })
        .collect();
    Some(default_version_name(mc_version, &loaders))
}

/// 现场查加载器最新版本（install 没给版本时用；也给 prompt 预填默认值）。
///
/// 抽成 trait 是为了让默认值逻辑能在没有网络的环境里测。
pub(crate) trait LoaderVersions: Send + Sync {
    fn latest<'a>(&'a self, kind: LoaderKind, mc_version: &'a str)
        -> BoxFuture<'a, Result<String>>;
}

/// 走下载源查最新版。
///
/// 三个 `fetch_*` 都按版本**从新到旧**返回，因此「最新」＝列表第一条；Fabric 额外
/// 优先稳定版（找不到稳定版才退回最新一条）。
impl LoaderVersions for DownloadSource {
    fn latest<'a>(
        &'a self,
        kind: LoaderKind,
        mc_version: &'a str,
    ) -> BoxFuture<'a, Result<String>> {
        Box::pin(async move {
            let version = match kind {
                LoaderKind::Fabric => {
                    let list = self.fetch_fabric_loaders(mc_version).await?;
                    list.iter()
                        .find(|entry| entry.loader.is_stable())
                        .or_else(|| list.first())
                        .map(|entry| entry.loader.version.clone())
                        .unwrap_or_default()
                }
                LoaderKind::Forge => self
                    .fetch_forge_versions(mc_version)
                    .await?
                    .first()
                    .map(dmcl_mcmeta::forge::ForgeVersionEntry::full_name)
                    .unwrap_or_default(),
                LoaderKind::NeoForge => self
                    .fetch_neoforge_versions(mc_version)
                    .await?
                    .first()
                    .map(|entry| entry.version.clone())
                    .unwrap_or_default(),
            };
            Ok(version)
        })
    }
}

/// `install` 命令的参数默认值（给 prompt 预填）。
pub(crate) struct InstallDefaults<L> {
    pub(crate) loaders: L,
}

impl<L: LoaderVersions> ArgDefaults for InstallDefaults<L> {
    fn default_for<'a>(
        &'a self,
        id: &'a str,
        answers: &'a BTreeMap<String, String>,
        enabled_flags: &'a [String],
    ) -> BoxFuture<'a, Option<String>> {
        Box::pin(async move {
            // 加载器版本：按已经填好的 mc 版本现场查最新
            if let Some(kind) = loader_version_arg(id) {
                let mc_version = answers.get("version_id")?;
                return match self.loaders.latest(kind, mc_version).await {
                    Ok(version) => Some(version),
                    Err(err) => {
                        eprintln!(
                            "提示：无法查询 {} 最新版本（{err:#}），请手动输入。",
                            kind.display_name()
                        );
                        None
                    }
                };
            }
            // 版本名：按 mc 版本 + 已启用的加载器拼出默认名
            if id == "version_name" {
                return default_version_name_for_prompt(answers, enabled_flags);
            }
            None
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn loader_arguments_map_to_loader_kinds() {
        assert_eq!(loader_version_arg("forge_version"), Some(LoaderKind::Forge));
        assert_eq!(
            loader_version_arg("neoforge_version"),
            Some(LoaderKind::NeoForge)
        );
        assert_eq!(
            loader_version_arg("fabric_version"),
            Some(LoaderKind::Fabric)
        );
        assert_eq!(loader_version_arg("version_id"), None);

        assert_eq!(loader_flag_arg("forge"), Some(LoaderKind::Forge));
        assert_eq!(loader_flag_arg("fabric"), Some(LoaderKind::Fabric));
        assert_eq!(loader_flag_arg("version_name"), None);
    }

    #[test]
    fn prompt_answers_compose_the_default_version_name() {
        let mut answers = BTreeMap::new();
        answers.insert("version_id".to_string(), "1.20.1".to_string());
        answers.insert("forge_version".to_string(), "47.2.0".to_string());

        // 只算已启用、且是加载器的开关
        let flags = vec!["forge".to_string(), "version_name".to_string()];
        assert_eq!(
            default_version_name_for_prompt(&answers, &flags).as_deref(),
            Some("1.20.1-forge-47.2.0")
        );

        // 没有 mc 版本 ⇒ None
        assert_eq!(
            default_version_name_for_prompt(&BTreeMap::new(), &flags),
            None
        );
    }

    struct FakeLoaders;

    impl LoaderVersions for FakeLoaders {
        fn latest<'a>(
            &'a self,
            kind: LoaderKind,
            _mc_version: &'a str,
        ) -> BoxFuture<'a, Result<String>> {
            Box::pin(async move { Ok(format!("{}-latest", kind.as_str())) })
        }
    }

    #[tokio::test]
    async fn install_defaults_resolve_loader_version_and_version_name() {
        let defaults = InstallDefaults {
            loaders: FakeLoaders,
        };
        let mut answers = BTreeMap::new();
        answers.insert("version_id".to_string(), "1.20.1".to_string());
        let flags = vec!["forge".to_string()];

        // 加载器版本参数 ⇒ 现场查最新
        assert_eq!(
            defaults
                .default_for("forge_version", &answers, &flags)
                .await
                .as_deref(),
            Some("forge-latest")
        );
        // 版本名 ⇒ 按 mc + 已启用加载器拼（版本还没填 ⇒ 不带版本号）
        assert_eq!(
            defaults
                .default_for("version_name", &answers, &flags)
                .await
                .as_deref(),
            Some("1.20.1-forge")
        );
        // 无关参数 ⇒ 没有默认值
        assert_eq!(
            defaults.default_for("window_width", &answers, &flags).await,
            None
        );

        // 填上加载器版本后再拼
        answers.insert("forge_version".to_string(), "47.2.0".to_string());
        assert_eq!(
            defaults
                .default_for("version_name", &answers, &flags)
                .await
                .as_deref(),
            Some("1.20.1-forge-47.2.0")
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
