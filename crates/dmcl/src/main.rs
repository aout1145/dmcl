//! `dmcl` 二进制的入口。
//!
//! 只做三件事：读配置、解析参数、交给 [`dmcl::cli::run`]。

use dmcl::cli::{run, Cli};
use dmcl::config::Config;

#[tokio::main]
async fn main() {
    if let Err(err) = real_main().await {
        eprintln!("错误：{err:#}");
        std::process::exit(1);
    }
}

async fn real_main() -> anyhow::Result<()> {
    let cli = Cli::parse_localized();
    let mut config = Config::load(None).await?;
    run(cli, &mut config).await
}
