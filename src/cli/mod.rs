//! Command line interface (was `dmcl.cli`).
//!
//! Two pieces of hand-written code from the D implementation are replaced by
//! mature crates:
//!
//! * argument parsing / `--help` generation — the UDA + `__traits` reflection
//!   parser is gone, [`clap`] derive handles it;
//! * the terminal menu — the raw `getch()` + ANSI TUI is gone, [`dialoguer`]
//!   handles it.
//!
//! All command semantics are preserved.

mod prompt;

use std::ffi::OsString;
use std::sync::Arc;

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::account::OfflineAccount;
use crate::config::{get_config_path, show_config, with_config, with_config_mut};
use crate::download::mirrors::Mirror;
use crate::download::{
    download_game_files, download_vanilla, get_latest_fabric, get_latest_forge,
    get_latest_neoforge, install_fabric, install_forge, install_neoforge, show_fabric_version_list,
    show_forge_version_list, show_neoforge_version_list, show_version_list,
};
use crate::env::{LAUNCHER_NAME, LAUNCHER_VERSION};
use crate::launch::{find_java, GameLauncher, LaunchOption};

pub use prompt::prompt_interactive;

/// `dmcl` — an unofficial Minecraft launcher.
#[derive(Parser, Debug)]
#[command(name = "dmcl", version = env!("CARGO_PKG_VERSION"), about, long_about = None)]
pub struct Cli {
    /// select the java to use and remember it as a named java config
    #[arg(long, global = true, value_name = "JAVA")]
    pub java: Option<String>,

    /// download mirror
    #[arg(long, global = true, value_name = "MIRROR")]
    pub mirror: Option<Mirror>,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

/// Every command of the original `DmclCli`.
#[derive(Subcommand, Debug)]
pub enum Commands {
    /// launch game
    Launch {
        #[arg(value_name = "version-name")]
        vername: String,
    },

    /// install game
    Install {
        #[arg(value_name = "mc-version-id")]
        version_id: String,
        #[arg(value_name = "version-name")]
        version_name: String,
        /// also install forge
        #[arg(long)]
        forge: bool,
        /// also install neoforge
        #[arg(long)]
        neoforge: bool,
        /// also install fabric
        #[arg(long)]
        fabric: bool,
    },

    /// show minecraft version list
    ListMc {
        #[arg(
            long = "type",
            value_name = "type",
            value_delimiter = ',',
            default_value = "release"
        )]
        r#type: Vec<String>,
    },

    /// show forge version list
    ListForge {
        #[arg(value_name = "mc-version-id")]
        verid: String,
    },

    /// show neoforge version list
    #[command(name = "list-neoforge")]
    ListNeoforge {
        #[arg(value_name = "mc-version-id")]
        verid: String,
    },

    /// show fabric version list
    ListFabric {
        #[arg(value_name = "mc-version-id")]
        verid: String,
    },

    /// install forge on version
    InstallForge {
        #[arg(value_name = "version-name")]
        vername: String,
        #[arg(value_name = "forge-version-id")]
        forgever: String,
    },

    /// install neoforge on version
    #[command(name = "install-neoforge")]
    InstallNeoforge {
        #[arg(value_name = "version-name")]
        vername: String,
        #[arg(value_name = "neoforge-version-id")]
        forgever: String,
    },

    /// search java automatically
    SearchJava,

    /// interactive prompt
    Prompt,

    /// show version
    Version,

    /// show config
    #[command(name = "config")]
    ShowConfig,

    /// edit config
    EditConfig,
}

/// Entry point of the CLI front-end.
pub async fn run(cli: Cli) -> Result<()> {
    // `preProcessArguments`: `--java` and `--mirror` are applied before the
    // command runs (they only mutate the in-memory config, like the original).
    if let Some(java) = cli.java.clone() {
        with_config_mut(|c| {
            // the java *path* doubles as the config entry name, exactly like
            // the D `preProcessArguments` did
            c.launch
                .java_configs
                .insert(java.clone(), std::path::PathBuf::from(&java));
            c.launch.java = crate::config::JavaChoice::Named(java);
        });
    }
    if let Some(mirror) = cli.mirror {
        with_config_mut(|c| {
            c.download.mirror = mirror;
        });
    }

    match cli.command {
        Some(command) => dispatch(command).await,
        None => {
            // No subcommand ⇒ run `cli.default_command` (default: `prompt`).
            let default = with_config(|c| c.cli.default_command.clone());
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
                    Some(command) => dispatch(command).await,
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

async fn dispatch(command: Commands) -> Result<()> {
    let mirror = with_config(|c| c.download.mirror);
    let root_path = with_config(|c| c.launch.minecraft_root_path.clone());

    match command {
        Commands::Launch { vername } => {
            download_game_files(mirror, &root_path, &vername).await?;
            let username = with_config(|c| c.account.username.clone());
            let mut option = LaunchOption::new(
                Arc::new(OfflineAccount::new(username)),
                root_path.clone(),
                vername,
            );
            with_config(|c| {
                option.window_width = c.launch.window_width;
                option.window_height = c.launch.window_height;
                option.min_memory = c.launch.min_memory;
                option.max_memory = c.launch.max_memory;
                option.custom_info = c.launch.custom_info.clone();
                option.additional_jvm = c.launch.additional_jvm.clone();
                option.additional_game = c.launch.additional_game.clone();
            });
            let mut launcher = GameLauncher::new(option).await?;
            let mut monitor = launcher.launch_game().await?;
            monitor.wait().await?;
        }

        Commands::Install {
            version_id,
            version_name,
            forge,
            neoforge,
            fabric,
        } => {
            download_vanilla(mirror, &root_path, &version_id, &version_name).await?;
            if forge {
                let forgever = get_latest_forge(mirror, &version_id).await?;
                install_forge(mirror, &root_path, &version_name, &forgever).await?;
            }
            if neoforge {
                let forgever = get_latest_neoforge(mirror, &version_id).await?;
                install_neoforge(mirror, &root_path, &version_name, &forgever).await?;
            }
            if fabric {
                let fabricver = get_latest_fabric(mirror, &version_id).await?;
                install_fabric(mirror, &root_path, &version_name, &fabricver).await?;
            }
            download_game_files(mirror, &root_path, &version_name).await?;
        }

        Commands::ListMc { r#type } => show_version_list(mirror, &r#type).await?,

        Commands::ListForge { verid } => show_forge_version_list(mirror, &verid).await?,

        Commands::ListNeoforge { verid } => show_neoforge_version_list(mirror, &verid).await?,

        Commands::ListFabric { verid } => show_fabric_version_list(mirror, &verid).await?,

        Commands::InstallForge { vername, forgever } => {
            install_forge(mirror, &root_path, &vername, &forgever).await?;
            download_game_files(mirror, &root_path, &vername).await?;
        }

        Commands::InstallNeoforge { vername, forgever } => {
            install_neoforge(mirror, &root_path, &vername, &forgever).await?;
            download_game_files(mirror, &root_path, &vername).await?;
        }

        Commands::SearchJava => find_java().await?,

        Commands::Prompt => prompt_interactive().await?,

        Commands::Version => {
            println!("{LAUNCHER_NAME} version {LAUNCHER_VERSION}");
            println!("build time: {}", env!("DMCL_BUILD_TIME"));
            println!();
            println!("thanks: bmclapi");
        }

        Commands::ShowConfig => show_config(),

        Commands::EditConfig => println!("Tip: edit {}", get_config_path().display()),
    }
    Ok(())
}
