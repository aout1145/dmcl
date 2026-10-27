//! 命令行前端。
//!
//! 参数解析 / `--help` 交给 [`clap`] derive；任务警告的「是否继续？」确认用
//! [`dialoguer`]。
//!
//! 前端的职责只有三件：把配置翻译成各 crate 的 `*Options`、打印事件流、显示结果
//! —— 业务逻辑都在 `dmcl-*` 里。命令定义、本地化与分发留在本模块；
//! 各命令的实现按文件拆在 `commands` 子模块下。

mod commands;
mod progress;

use std::path::Path;

use anyhow::{anyhow, Context, Result};
use clap::error::{ContextKind, ErrorKind};
use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};

use crate::config::{Config, ConfigError};

/// 非官方 Minecraft 启动器。
#[derive(Parser, Debug)]
#[command(name = "dmcl", version = env!("CARGO_PKG_VERSION"), long_about = None)]
pub struct Cli {
    /// 本次运行使用的 Minecraft 根目录（`minecraft_roots` 里的名字）
    #[arg(long, global = true, value_name = "name")]
    pub minecraft_root: Option<String>,

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

/// 没有位置参数的命令（`list-mc`、`list` …）—— 不打出空的小节。
const HELP_TEMPLATE_WITHOUT_POSITIONALS: &str =
    "{about-with-newline}\n用法：{usage}\n\n选项：\n{options}{after-help}";

/// 带子命令的命令（根命令）—— 额外列出子命令。
const HELP_TEMPLATE_WITH_SUBCOMMANDS: &str =
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

/// 把 clap 自动生成的英文说明换成中文（递归处理子命令）。
fn localized_command() -> clap::Command {
    fn localize(mut command: clap::Command) -> clap::Command {
        // `--help` / `--version` / `help` 子命令是 clap 在 build 阶段补进命令树的，
        // 先 build 一次才能改到它们
        command.build();
        let has_subcommands = command.get_subcommands().next().is_some();
        let has_positionals = command.get_positionals().next().is_some();
        let template = if has_subcommands {
            HELP_TEMPLATE_WITH_SUBCOMMANDS
        } else if has_positionals {
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

        // 递归处理子命令（`help` 也要中文化）
        let names: Vec<String> = command
            .get_subcommands()
            .map(|sub| sub.get_name().to_string())
            .collect();
        for name in names {
            if name == "help" {
                command = command.mut_subcommand("help", |sub| {
                    sub.about("打印帮助信息（也可查询某个子命令的帮助）")
                });
            } else {
                command = command.mut_subcommand(name, localize);
            }
        }
        command
    }

    localize(Cli::command())
}

/// 打印中文帮助；无 GUI 且无子命令时用。
pub fn print_help() -> Result<()> {
    localized_command()
        .print_help()
        .context("无法打印帮助信息")?;
    println!();
    Ok(())
}

/// 全部子命令。
#[derive(Subcommand, Debug)]
pub enum Commands {
    /// 启动游戏
    Launch {
        /// 本地版本名
        #[arg(value_name = "version-name")]
        vername: String,
    },

    /// 安装游戏
    Install {
        /// Minecraft 版本号
        #[arg(value_name = "mc-version-id")]
        version_id: String,
        /// 安装 Forge
        #[arg(long)]
        forge: bool,
        /// 安装 NeoForge
        #[arg(long)]
        neoforge: bool,
        /// 安装 Fabric
        #[arg(long)]
        fabric: bool,
        /// Forge 版本
        #[arg(long, value_name = "version")]
        forge_version: Option<String>,
        /// NeoForge 版本
        #[arg(long, value_name = "version")]
        neoforge_version: Option<String>,
        /// Fabric loader 版本
        #[arg(long, value_name = "version")]
        fabric_version: Option<String>,
        /// 本地版本名
        #[arg(value_name = "version-name")]
        version_name: Option<String>,
    },

    /// 列出已安装的版本
    List,

    /// 列出 Minecraft 版本
    ListMc {
        /// 版本类型，逗号分隔
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
        /// Minecraft 版本号
        #[arg(value_name = "mc-version-id")]
        verid: String,
    },

    /// 列出 NeoForge 版本
    #[command(name = "list-neoforge")]
    ListNeoforge {
        /// Minecraft 版本号
        #[arg(value_name = "mc-version-id")]
        verid: String,
    },

    /// 列出 Fabric 版本
    ListFabric {
        /// Minecraft 版本号
        #[arg(value_name = "mc-version-id")]
        verid: String,
    },

    /// 搜索本机 java（常见安装目录与 JAVA_HOME）并写入配置
    #[command(name = "search-java")]
    SearchJava,

    /// 启动图形界面
    Gui {
        /// 前端名称（默认 gtk4）
        #[arg(value_name = "frontend")]
        frontend: Option<String>,
    },

    /// 显示版本
    Version,
}

/// 执行一个命令。
///
/// 配置加载有问题时（`error` 非空），CLI 把每条问题作为错误打到 stderr 并结束，
/// 不继续执行命令。
pub async fn dispatch(
    command: Commands,
    config: &mut Config,
    error: Option<ConfigError>,
    root_path: &Path,
) -> Result<()> {
    if let Some(error) = error {
        for warning in &error.warnings {
            eprintln!("错误：{warning}");
        }
        return Err(error.into());
    }

    match command {
        // GUI 必须在主线程启动（见 `main.rs`），不该走这条 tokio 路径。
        Commands::Gui { .. } => Err(anyhow!("gui 前端必须在主线程启动")),

        Commands::Launch { vername } => commands::launch::run(&vername, config, root_path).await,

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
            let args = commands::install::InstallArgs {
                version_id,
                version_name,
                forge,
                neoforge,
                fabric,
                forge_version,
                neoforge_version,
                fabric_version,
            };
            commands::install::run(args, config, root_path).await
        }

        Commands::ListMc { r#type } => commands::list::mc(config, &r#type).await,
        Commands::ListForge { verid } => commands::list::forge(config, &verid).await,
        Commands::ListNeoforge { verid } => commands::list::neoforge(config, &verid).await,
        Commands::ListFabric { verid } => commands::list::fabric(config, &verid).await,

        Commands::List => commands::list::installed(root_path).await,

        Commands::SearchJava => commands::search_java::run(config).await,

        Commands::Version => commands::version::run().await,
    }
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
            "list",
            "search-java",
            "gui",
            "version",
        ] {
            assert!(names.contains(&expected), "缺少命令 {expected}: {names:?}");
        }
        // 删掉的命令不该再出现
        for removed in [
            "install-forge",
            "install-neoforge",
            "edit-config",
            "prompt",
            "config",
            "java",
        ] {
            assert!(
                !names.contains(&removed),
                "不该还有命令 {removed}: {names:?}"
            );
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
    fn install_version_name_is_optional() {
        let cli = Cli::try_parse_from(["dmcl", "install", "1.20.1"]).unwrap();
        match cli.command {
            Some(Commands::Install { version_name, .. }) => assert!(version_name.is_none()),
            other => panic!("解析结果不对: {other:?}"),
        }

        let cli = Cli::try_parse_from(["dmcl", "install", "1.20.1", "my-name"]).unwrap();
        match cli.command {
            Some(Commands::Install { version_name, .. }) => {
                assert_eq!(version_name.as_deref(), Some("my-name"));
            }
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

    #[test]
    fn search_java_takes_no_arguments() {
        let cli = Cli::try_parse_from(["dmcl", "search-java"]).unwrap();
        assert!(matches!(cli.command, Some(Commands::SearchJava)));

        let err = Cli::try_parse_from(["dmcl", "search-java", "extra"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::UnknownArgument);
    }

    #[test]
    fn minecraft_root_is_a_global_option() {
        let before =
            Cli::try_parse_from(["dmcl", "--minecraft-root", "test", "launch", "1.20.1"]).unwrap();
        assert_eq!(before.minecraft_root.as_deref(), Some("test"));
        assert!(matches!(
            before.command,
            Some(Commands::Launch { vername }) if vername == "1.20.1"
        ));

        let after =
            Cli::try_parse_from(["dmcl", "launch", "1.20.1", "--minecraft-root=test"]).unwrap();
        assert_eq!(after.minecraft_root.as_deref(), Some("test"));
        assert!(matches!(
            after.command,
            Some(Commands::Launch { vername }) if vername == "1.20.1"
        ));
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

        // 根命令要列出子命令
        assert!(root_help.contains("命令："));
        assert!(root_help.contains("search-java"));
        for removed in ["prompt", "config"] {
            assert!(
                !root_help.contains(removed),
                "帮助里残留 {removed}:\n{root_help}"
            );
        }
        assert!(root.find_subcommand("java").is_none());
    }

    /// clap 的报错也要是中文。
    #[test]
    fn clap_errors_are_translated() {
        let command = localized_command();

        let err = command
            .clone()
            .try_get_matches_from(["dmcl", "list-forge"])
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
