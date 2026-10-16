//! `dmcl` 二进制的入口。
//!
//! 步骤：解析参数 → 启动时同步读一次配置 → 决定走 GUI 还是命令行。
//!
//! GUI 必须占主线程跑 GLib 主循环，所以这里**不用** `#[tokio::main]`：先同步地把命令
//! 定下来，走 GUI 就直接启动，其余命令再建 tokio runtime 执行。

use dmcl::cli::{self, Cli, Commands};
use dmcl::config::Config;

fn main() {
    if let Err(err) = real_main() {
        eprintln!("错误：{err:#}");
        std::process::exit(1);
    }
}

fn real_main() -> anyhow::Result<()> {
    let cli = Cli::parse_localized();
    // 启动时同步读一次配置：据此解析默认命令，也据此决定走哪条前端路径。
    let mut config = Config::load_sync(None)?;
    let command = cli::resolve_command(cli, &config)?;

    match command {
        Commands::Gui { frontend } => dmcl::gui::run(frontend, config),
        command => {
            let runtime = tokio::runtime::Runtime::new()?;
            runtime.block_on(cli::dispatch(command, &mut config))
        }
    }
}
