//! 命令行前端（原 `dmcl.cli`）。
//!
//! 两处 D 手写代码换成了成熟库：
//!
//! * 参数解析 / `--help` —— UDA + `__traits` 反射解析器没了，交给 [`clap`] derive；
//! * 终端菜单 —— 原生 `getch()` + ANSI 界面没了，交给 [`dialoguer`]。
//!
//! 命令语义保持原样。前端的职责只有三件：把配置翻译成各 crate 的 `*Options`、
//! 打印事件流、显示结果 —— 业务逻辑都在 `dmcl-*` 里。

mod progress;
mod prompt;

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use clap::{Parser, Subcommand};

use dmcl_install::loader::fabric_loaders;
use dmcl_install::{
    forge_versions, install_loader_task, install_task, neoforge_versions, InstallSpec, LoaderKind,
    LoaderRequest,
};
use dmcl_launch::{
    find_java, GameLauncher, JavaChoice, DEFAULT_JAVA_ROOTS, LAUNCHER_NAME, LAUNCHER_VERSION,
};
use dmcl_mcmeta::Mirror;

use crate::config::Config;

use progress::ProgressPrinter;
pub use prompt::prompt_interactive;

/// `dmcl` —— 非官方 Minecraft 启动器。
#[derive(Parser, Debug)]
#[command(name = "dmcl", version = env!("CARGO_PKG_VERSION"), about, long_about = None)]
pub struct Cli {
    /// 用这个 java，并把它记进命名 java 配置
    #[arg(long, global = true, value_name = "JAVA")]
    pub java: Option<String>,

    /// 下载镜像源
    #[arg(long, global = true, value_name = "MIRROR")]
    pub mirror: Option<Mirror>,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

/// 原 `DmclCli` 的全部命令。
#[derive(Subcommand, Debug)]
pub enum Commands {
    /// 启动游戏
    Launch {
        #[arg(value_name = "version-name")]
        vername: String,
    },

    /// 安装游戏
    Install {
        #[arg(value_name = "mc-version-id")]
        version_id: String,
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
        #[arg(
            long = "type",
            value_name = "type",
            value_delimiter = ',',
            default_value = "release"
        )]
        r#type: Vec<String>,
    },

    /// 列出 Forge 版本
    ListForge {
        #[arg(value_name = "mc-version-id")]
        verid: String,
    },

    /// 列出 NeoForge 版本
    #[command(name = "list-neoforge")]
    ListNeoforge {
        #[arg(value_name = "mc-version-id")]
        verid: String,
    },

    /// 列出 Fabric 版本
    ListFabric {
        #[arg(value_name = "mc-version-id")]
        verid: String,
    },

    /// 在指定版本上安装 Forge
    InstallForge {
        #[arg(value_name = "version-name")]
        vername: String,
        #[arg(value_name = "forge-version-id")]
        forgever: String,
    },

    /// 在指定版本上安装 NeoForge
    #[command(name = "install-neoforge")]
    InstallNeoforge {
        #[arg(value_name = "version-name")]
        vername: String,
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
    // `preProcessArguments`：`--java` 与 `--mirror` 在命令执行前生效
    // （和原来一样，只改内存里的配置）
    if let Some(java) = cli.java.clone() {
        // java 的**路径**同时充当配置项的名字，与原 `preProcessArguments` 一致
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
                println!("unknown command");
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
                        println!("unknown command");
                        Ok(())
                    }
                },
                Err(_) => {
                    println!("unknown command");
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
                .run_with(|event| printer.handle(event))
                .await;
            printer.finish();
            result
        }

        Commands::ListMc { r#type } => {
            let manifest = source.fetch_version_manifest().await?;
            // 与原实现一致：从新到旧
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
            println!("{LAUNCHER_NAME} version {LAUNCHER_VERSION}");
            println!("build time: {}", env!("DMCL_BUILD_TIME"));
            println!();
            println!("thanks: bmclapi");
            Ok(())
        }

        Commands::ShowConfig => {
            config.show();
            Ok(())
        }

        Commands::EditConfig => {
            println!("Tip: edit {}", Config::path().display());
            Ok(())
        }
    }
}

/// `launch` —— 先校验完整性，缺文件就补（由前端决定去调安装），然后启动。
async fn launch(vername: &str, config: &mut Config) -> Result<()> {
    let source = config.download_source();
    let option = config.launch_option(config.account(), vername);
    let mut launcher = GameLauncher::new(option).await?;

    let status = launcher.check_integrity().await?;
    if !status.is_complete() {
        println!("{}", status.summary());
        println!("{}", status.detail(10));
        // 修复是调用者的事：这里显式跑一次"下载游戏文件"（已存在的文件会被跳过）
        let options = Arc::new(config.install_options(source));
        let mut printer = ProgressPrinter::new();
        let result = dmcl_install::game_files_task(options, vername.to_string())
            .run_with(|event| printer.handle(event))
            .await;
        printer.finish();
        result?;
    }

    let mut monitor = launcher.launch_game().await?;
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
        .run_with(|event| printer.handle(event))
        .await;
    printer.finish();
    result
}

/// `search-java` —— 扫描并**写回配置**（这是前端才有权做的事）。
async fn search_java(config: &mut Config) -> Result<()> {
    println!("find following javas:");
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
}
