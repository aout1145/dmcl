//! `dmcl` 二进制的入口。
//!
//! 步骤：解析参数 → 同步读一次配置 → 无子命令走默认路径（GUI 或 prompt），其余命令分发。
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
    let parsed = Cli::parse_localized();
    // 启动时同步读一次配置：GUI 必须在建窗口 / GLib 主循环之前拿到它。
    let mut config = Config::load_sync(None)?;

    match parsed.command {
        // 无子命令：有 GUI 前端就启动 GUI，没有就进交互菜单。
        None => dmcl::gui::run_default(config),
        // 显式 `gui`：前端不可用时直接报错，不回退。
        Some(Commands::Gui { frontend }) => dmcl::gui::run(frontend, config),
        Some(command) => {
            let runtime = tokio::runtime::Runtime::new()?;
            runtime.block_on(cli::dispatch(command, &mut config))
        }
    }
}
