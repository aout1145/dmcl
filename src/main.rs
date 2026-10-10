//! Entry point of the `dmcl` binary — mirrors `dmcl.main`.

use clap::Parser;

use dmcl::cli::{run, Cli};
use dmcl::config::read_config;

#[tokio::main]
async fn main() {
    if let Err(err) = real_main().await {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}

async fn real_main() -> anyhow::Result<()> {
    debug_snapshot();
    read_config(None).await?;
    let cli = Cli::parse();
    run(cli).await
}

#[cfg(debug_assertions)]
fn debug_snapshot() {
    println!("!!! DMCL SNAPSHOT !!!\n");
}

#[cfg(not(debug_assertions))]
fn debug_snapshot() {}
