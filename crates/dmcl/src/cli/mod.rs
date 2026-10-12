//! 命令行前端。
//!
//! 参数解析 / `--help` 交给 [`clap`] derive；交互式菜单交给 [`dialoguer`]。
//!
//! 前端的职责只有三件：把配置翻译成各 crate 的 `*Options`、打印事件流、显示结果
//! —— 业务逻辑都在 `dmcl-*` 里。

mod progress;
mod prompt;

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use clap::error::{ContextKind, ErrorKind};
use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};
use dialoguer::Confirm;

use dmcl_install::{install_loader_task, install_task, InstallSpec, LoaderKind, LoaderRequest};
use dmcl_launch::{find_java, JavaChoice, DEFAULT_JAVA_ROOTS, LAUNCHER_NAME, LAUNCHER_VERSION};
use dmcl_mcmeta::fabric::fabric_loaders;
use dmcl_mcmeta::forge::forge_versions;
use dmcl_mcmeta::neoforge::neoforge_versions;
use dmcl_mcmeta::source::Mirror;
use dmcl_task::TaskEvent;

use crate::config::Config;

use progress::ProgressPrinter;
pub use prompt::prompt_interactive;

/// 非官方 Minecraft 启动器。
#[derive(Parser, Debug)]
#[command(name = "dmcl", version = env!("CARGO_PKG_VERSION"), long_about = None)]
pub struct Cli {
    /// 用这个 java，并把它记进命名 java 配置
    #[arg(long, global = true, value_name = "JAVA")]
    pub java: Option<String>,

    /// 下载镜像源（official 或 bmclapi）
    #[arg(
        long,
        global = true,
        value_name = "MIRROR",
        hide_possible_values = true
    )]
    pub mirror: Option<Mirror>,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

// ---------------------------------------------------------------------------
// 全中文的帮助输出
// ---------------------------------------------------------------------------
//
// `clap` 自动生成的小节标题（`Usage:` / `Arguments:` / `Options:`）和
// `--help` / `--version` 的说明都是英文的，只能在构建命令树时逐层替换。
// 参数定义仍然只有一份 —— 就是 [`Cli`] 与 [`Commands`] 上的那些属性。

/// 有位置参数的命令（`install`、`list-forge` …）。
const HELP_TEMPLATE_WITH_POSITIONALS: &str =
    "{about-with-newline}\n用法：{usage}\n\n位置参数：\n{positionals}\n\n选项：\n{options}{after-help}";

/// 没有位置参数的命令（`list-mc`、`search-java` …）—— 不打出空的小节。
const HELP_TEMPLATE_WITHOUT_POSITIONALS: &str =
    "{about-with-newline}\n用法：{usage}\n\n选项：\n{options}{after-help}";

/// 根命令：额外列出子命令。
const HELP_TEMPLATE_ROOT: &str =
    "{about-with-newline}\n用法：{usage}\n\n选项：\n{options}\n\n命令：\n{subcommands}{after-help}";

impl Cli {
    /// 解析命令行，并让帮助文本与常见错误提示保持全中文。
    ///
    /// 与 `Cli::parse()` 的区别只有两处：先把命令树里的英文小节标题与
    /// `--help` / `--version` 说明替换掉，再把 clap 的英文报错翻成中文。
    pub fn parse_localized() -> Self {
        let command = localized_command();
        match command.try_get_matches() {
            Ok(matches) => Self::from_arg_matches(&matches).unwrap_or_else(|err| err.exit()),
            Err(err) => match err.kind() {
                // `--help` / `--version` 走的是"错误"通道，照常打印并退出
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => err.exit(),
                _ => {
                    match translate_error(&err) {
                        Some(message) => eprintln!("错误：{message}"),
                        // 认不出来的错误就照原样打印，免得把信息丢掉
                        None => eprint!("{err}"),
                    }
                    eprintln!("提示：用 dmcl --help 查看用法");
                    std::process::exit(2);
                }
            },
        }
    }
}

/// 把 clap 的英文报错翻成中文。
///
/// 只处理最常见的几种（少参数、参数不认识、命令不认识、取值不合法），
/// 其余返回 `None`，由调用者退回到 clap 原文。
fn translate_error(err: &clap::Error) -> Option<String> {
    let context = |kind: ContextKind| {
        err.get(kind)
            .map(|value| value.to_string().replace(", ", "、"))
            .unwrap_or_default()
    };
    match err.kind() {
        ErrorKind::MissingRequiredArgument => Some(format!(
            "缺少必填参数：{}",
            context(ContextKind::InvalidArg)
        )),
        ErrorKind::UnknownArgument => Some(format!(
            "不认识的参数：{}",
            context(ContextKind::InvalidArg)
        )),
        ErrorKind::InvalidSubcommand => Some(format!(
            "不认识的命令：{}",
            context(ContextKind::InvalidSubcommand)
        )),
        ErrorKind::InvalidValue => Some(format!(
            "参数取值不合法：{}",
            context(ContextKind::InvalidValue)
        )),
        ErrorKind::MissingSubcommand => Some("缺少子命令".to_string()),
        _ => None,
    }
}

/// 把 clap 自动生成的英文说明换成中文。
fn localized_command() -> clap::Command {
    fn localize(mut command: clap::Command) -> clap::Command {
        // `--help` / `--version` 是 clap 在 build 阶段补进命令树的，
        // 先 build 一次才能改到它们
        command.build();
        let has_positionals = command.get_positionals().next().is_some();
        let template = if has_positionals {
            HELP_TEMPLATE_WITH_POSITIONALS
        } else {
            HELP_TEMPLATE_WITHOUT_POSITIONALS
        };
        let mut command = command.help_template(template);
        // `mut_arg` 遇到不存在的参数会 panic，所以先问一句在不在
        if command.get_arguments().any(|arg| arg.get_id() == "help") {
            command = command.mut_arg("help", |arg| arg.help("打印帮助信息"));
        }
        if command.get_arguments().any(|arg| arg.get_id() == "version") {
            command = command.mut_arg("version", |arg| arg.help("打印版本信息"));
        }
        command
    }

    let root = Cli::command();
    // 先 build 一次：`--help` / `--version` / `help` 子命令都是 clap 在这个阶段补进去的
    let mut root = root;
    root.build();

    let names: Vec<String> = root
        .get_subcommands()
        .map(|sub| sub.get_name().to_string())
        .collect();

    root = localize(root).help_template(HELP_TEMPLATE_ROOT);
    for name in names {
        if name == "help" {
            // clap 自动加的 `help` 子命令，说明也得换掉
            root = root.mut_subcommand("help", |command| {
                command.about("打印帮助信息（也可查询某个子命令的帮助）")
            });
        } else {
            root = root.mut_subcommand(name, localize);
        }
    }
    root
}

/// 全部子命令。
#[derive(Subcommand, Debug)]
pub enum Commands {
    /// 启动游戏
    Launch {
        /// 本地版本目录名（versions/<名字>）
        #[arg(value_name = "version-name")]
        vername: String,
    },

    /// 安装游戏
    Install {
        /// Minecraft 版本号（如 1.20.1）
        #[arg(value_name = "mc-version-id")]
        version_id: String,
        /// 本地版本目录名（versions/<名字>）
        #[arg(value_name = "version-name")]
        version_name: String,
        /// 顺便装 Forge
        #[arg(long)]
        forge: bool,
        /// 顺便装 NeoForge
        #[arg(long)]
        neoforge: bool,
        /// 顺便装 Fabric
        #[arg(long)]
        fabric: bool,
        /// 指定 Forge 版本（默认取最新）
        #[arg(long, value_name = "version")]
        forge_version: Option<String>,
        /// 指定 NeoForge 版本（默认取最新）
        #[arg(long, value_name = "version")]
        neoforge_version: Option<String>,
        /// 指定 Fabric loader 版本（默认取最新稳定版）
        #[arg(long, value_name = "version")]
        fabric_version: Option<String>,
    },

    /// 列出 Minecraft 版本
    ListMc {
        /// 版本类型，逗号分隔（如 release,snapshot；默认 release）
        #[arg(
            long = "type",
            value_name = "type",
            value_delimiter = ',',
            default_value = "release",
            hide_default_value = true
        )]
        r#type: Vec<String>,
    },

    /// 列出 Forge 版本
    ListForge {
        /// Minecraft 版本号（如 1.20.1）
        #[arg(value_name = "mc-version-id")]
        verid: String,
    },

    /// 列出 NeoForge 版本
    #[command(name = "list-neoforge")]
    ListNeoforge {
        /// Minecraft 版本号（如 1.20.1）
        #[arg(value_name = "mc-version-id")]
        verid: String,
    },

    /// 列出 Fabric 版本
    ListFabric {
        /// Minecraft 版本号（如 1.20.1）
        #[arg(value_name = "mc-version-id")]
        verid: String,
    },

    /// 在指定版本上安装 Forge
    InstallForge {
        /// 本地版本目录名（versions/<名字>）
        #[arg(value_name = "version-name")]
        vername: String,
        /// Forge 版本号
        #[arg(value_name = "forge-version-id")]
        forgever: String,
    },

    /// 在指定版本上安装 NeoForge
    #[command(name = "install-neoforge")]
    InstallNeoforge {
        /// 本地版本目录名（versions/<名字>）
        #[arg(value_name = "version-name")]
        vername: String,
        /// NeoForge 版本号
        #[arg(value_name = "neoforge-version-id")]
        forgever: String,
    },

    /// 自动搜索本机 java
    SearchJava,

    /// 交互式菜单
    Prompt,

    /// 显示版本
    Version,

    /// 显示配置
    #[command(name = "config")]
    ShowConfig,

    /// 给出配置文件位置
    EditConfig,
}

/// 命令行前端的入口。
pub async fn run(cli: Cli, config: &mut Config) -> Result<()> {
    // `--java` 与 `--mirror` 在命令执行前生效（只改内存里的配置）
    if let Some(java) = cli.java.clone() {
        // java 的**路径**同时充当配置项的名字
        config
            .launch
            .java_configs
            .insert(java.clone(), PathBuf::from(&java));
        config.launch.java = JavaChoice::Named(java);
    }
    if let Some(mirror) = cli.mirror {
        config.download.mirror = mirror;
    }

    match cli.command {
        Some(command) => dispatch(command, config).await,
        None => {
            // 没有子命令 ⇒ 执行配置里的 `cli.default_command`（默认 `prompt`）
            let default = config.cli.default_command.clone();
            if default.is_empty() {
                println!("未知命令");
                return Ok(());
            }
            let argv: Vec<OsString> = vec![
                OsString::from(env!("CARGO_PKG_NAME")),
                OsString::from(default),
            ];
            match Cli::try_parse_from(argv) {
                Ok(parsed) => match parsed.command {
                    Some(command) => dispatch(command, config).await,
                    None => {
                        println!("未知命令");
                        Ok(())
                    }
                },
                Err(_) => {
                    println!("未知命令");
                    Ok(())
                }
            }
        }
    }
}

async fn dispatch(command: Commands, config: &mut Config) -> Result<()> {
    let source = config.download_source();

    match command {
        Commands::Launch { vername } => launch(&vername, config).await,

        Commands::Install {
            version_id,
            version_name,
            forge,
            neoforge,
            fabric,
            forge_version,
            neoforge_version,
            fabric_version,
        } => {
            let mut spec = InstallSpec::new(version_id, version_name);
            if forge {
                spec = spec.with_loader(LoaderKind::Forge, forge_version);
            }
            if neoforge {
                spec = spec.with_loader(LoaderKind::NeoForge, neoforge_version);
            }
            if fabric {
                spec = spec.with_loader(LoaderKind::Fabric, fabric_version);
            }
            let options = Arc::new(config.install_options(source));
            let mut printer = ProgressPrinter::new();
            let result = install_task(options, spec)
                .run_with((), |event| handle_event(&mut printer, event))
                .await;
            printer.finish();
            result
        }

        Commands::ListMc { r#type } => {
            let manifest = source.fetch_version_manifest().await?;
            // 从新到旧输出
            for version in manifest.filter_kinds(&r#type).rev() {
                println!("{} {}", version.kind, version.id);
            }
            Ok(())
        }

        Commands::ListForge { verid } => {
            for version in forge_versions(&source, &verid).await? {
                println!("{}", version.full_name());
            }
            Ok(())
        }

        Commands::ListNeoforge { verid } => {
            for version in neoforge_versions(&source, &verid).await? {
                println!("{version}");
            }
            Ok(())
        }

        Commands::ListFabric { verid } => {
            let loaders = fabric_loaders(&source, &verid).await?;
            for entry in loaders.iter().rev() {
                println!(
                    "{verid}/{}{}",
                    entry.loader.version,
                    if entry.loader.is_stable() {
                        "(recommanded)"
                    } else {
                        ""
                    }
                );
            }
            Ok(())
        }

        Commands::InstallForge { vername, forgever } => {
            install_loader(&vername, forgever, LoaderKind::Forge, config).await
        }

        Commands::InstallNeoforge { vername, forgever } => {
            install_loader(&vername, forgever, LoaderKind::NeoForge, config).await
        }

        Commands::SearchJava => search_java(config).await,

        Commands::Prompt => prompt_interactive().await,

        Commands::Version => {
            println!("{LAUNCHER_NAME} 版本 {LAUNCHER_VERSION}");
            println!("构建时间：{}", env!("DMCL_BUILD_TIME"));
            println!();
            println!("感谢：bmclapi");
            Ok(())
        }

        Commands::ShowConfig => {
            config.show();
            Ok(())
        }

        Commands::EditConfig => {
            println!("提示：编辑 {}", Config::path().display());
            Ok(())
        }
    }
}

/// 渲染一条任务事件；遇到**警告**就询问用户，让它决定继续还是中止。
fn handle_event(printer: &mut ProgressPrinter, event: TaskEvent) {
    let warning = match &event {
        TaskEvent::Warning {
            message, decision, ..
        } => Some((message.clone(), decision.clone())),
        _ => None,
    };
    printer.handle(event);

    if let Some((message, decision)) = warning {
        let proceed = if printer.is_interactive() {
            Confirm::new()
                .with_prompt(format!("⚠ {message}\n是否继续？"))
                .default(true)
                .interact()
                .unwrap_or(true)
        } else {
            true
        };
        if proceed {
            decision.accept();
        } else {
            decision.abort();
        }
    }
}

/// `launch` —— 检查完整性 →（缺件则补全）→ 生成启动参数 → 启动子进程，等游戏退出。
///
/// 整条「准备 → 补全 → 启动」链由 `dmcl-launch` 的 [`dmcl_launch::launch_chain`] 组装，
/// 这里负责显示进度，并在拿到**未 wait** 的 `GameMonitor` 后等它退出。
async fn launch(vername: &str, config: &mut Config) -> Result<()> {
    let source = config.download_source();
    let option = config.launch_option(config.account(), vername);
    let install = Arc::new(config.install_options(source));

    let mut printer = ProgressPrinter::new();
    let mut monitor = dmcl_launch::launch_chain(option, install)
        .run_with((), |event| handle_event(&mut printer, event))
        .await?;
    printer.finish();

    monitor.wait().await?;
    Ok(())
}

/// `install-forge` / `install-neoforge`：在已有版本上装加载器再补齐文件。
async fn install_loader(
    vername: &str,
    loader_version: String,
    kind: LoaderKind,
    config: &Config,
) -> Result<()> {
    let options = Arc::new(config.install_options(config.download_source()));
    let request = LoaderRequest::version(kind, loader_version);
    let mut printer = ProgressPrinter::new();
    let result = install_loader_task(options, vername.to_string(), vername.to_string(), request)
        .run_with((), |event| handle_event(&mut printer, event))
        .await;
    printer.finish();
    result
}

/// `search-java` —— 扫描并**写回配置**（这是前端才有权做的事）。
async fn search_java(config: &mut Config) -> Result<()> {
    println!("找到以下 java：");
    let configs = find_java(DEFAULT_JAVA_ROOTS).await?;
    for (name, path) in &configs {
        println!("{name}: {}", path.display());
    }
    config.launch.java_configs = configs;
    config.launch.java = JavaChoice::Auto;
    config.save().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_keep_their_original_names() {
        use clap::CommandFactory;
        let root = Cli::command();
        let names: Vec<&str> = root.get_subcommands().map(|c| c.get_name()).collect();
        for expected in [
            "launch",
            "install",
            "list-mc",
            "list-forge",
            "list-neoforge",
            "list-fabric",
            "install-forge",
            "install-neoforge",
            "search-java",
            "prompt",
            "version",
            "config",
            "edit-config",
        ] {
            assert!(names.contains(&expected), "缺少命令 {expected}: {names:?}");
        }
    }

    #[test]
    fn list_mc_defaults_to_release() {
        let cli = Cli::try_parse_from(["dmcl", "list-mc"]).unwrap();
        match cli.command {
            Some(Commands::ListMc { r#type }) => assert_eq!(r#type, vec!["release"]),
            other => panic!("解析结果不对: {other:?}"),
        }
    }

    #[test]
    fn global_options_are_accepted_in_any_position() {
        let cli = Cli::try_parse_from([
            "dmcl",
            "--mirror",
            "bmclapi",
            "launch",
            "--java",
            "/opt/java/bin/java",
            "1.20.1",
        ])
        .unwrap();
        assert_eq!(cli.mirror, Some(Mirror::Bmclapi));
        assert_eq!(cli.java.as_deref(), Some("/opt/java/bin/java"));
        match cli.command {
            Some(Commands::Launch { vername }) => assert_eq!(vername, "1.20.1"),
            other => panic!("解析结果不对: {other:?}"),
        }
    }

    #[test]
    fn install_flags_and_versions_parse() {
        let cli = Cli::try_parse_from([
            "dmcl",
            "install",
            "1.20.1",
            "1.20.1-forge",
            "--forge",
            "--forge-version",
            "47.2.0",
        ])
        .unwrap();
        match cli.command {
            Some(Commands::Install {
                forge,
                forge_version,
                neoforge,
                ..
            }) => {
                assert!(forge);
                assert!(!neoforge);
                assert_eq!(forge_version.as_deref(), Some("47.2.0"));
            }
            other => panic!("解析结果不对: {other:?}"),
        }
    }

    /// 帮助文本里不该再出现 clap 的英文小节标题。
    #[test]
    fn help_output_is_chinese() {
        let root = localized_command();

        let root_help = root.clone().render_long_help().to_string();
        for english in ["Usage:", "Options:", "Commands:", "Print help"] {
            assert!(
                !root_help.contains(english),
                "根帮助里残留 {english}:\n{root_help}"
            );
        }
        assert!(root_help.contains("用法："));
        assert!(root_help.contains("打印帮助信息"));

        let install_help = root
            .find_subcommand("install")
            .unwrap()
            .clone()
            .render_long_help()
            .to_string();
        for english in ["Usage:", "Arguments:", "Options:", "Print help"] {
            assert!(
                !install_help.contains(english),
                "install 帮助里残留 {english}:\n{install_help}"
            );
        }
        assert!(install_help.contains("位置参数："));
        assert!(install_help.contains("Minecraft 版本号"));
    }

    /// clap 的报错也要是中文。
    #[test]
    fn clap_errors_are_translated() {
        let command = localized_command();

        let err = command
            .clone()
            .try_get_matches_from(["dmcl", "install"])
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::MissingRequiredArgument);
        let message = translate_error(&err).expect("应当翻译成中文");
        assert!(message.contains("缺少必填参数"), "{message}");
        assert!(message.contains("mc-version-id"), "{message}");

        let err = command
            .clone()
            .try_get_matches_from(["dmcl", "--nope"])
            .unwrap_err();
        assert!(translate_error(&err).unwrap().contains("不认识的参数"));

        let err = command
            .clone()
            .try_get_matches_from(["dmcl", "bogus"])
            .unwrap_err();
        assert!(translate_error(&err).unwrap().contains("不认识的命令"));

        // `--help` 走的是"错误"通道，但绝不能被当成报错翻译
        let err = command
            .clone()
            .try_get_matches_from(["dmcl", "--help"])
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::DisplayHelp);
        assert!(translate_error(&err).is_none());
    }
}
