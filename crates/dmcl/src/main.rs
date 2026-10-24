//! `dmcl` 二进制的入口。
//!
//! 步骤：解析参数 → 同步读一次配置 → 无子命令走默认路径（GUI 或帮助），其余命令分发。
//!
//! GUI 必须占主线程跑 GLib 主循环，所以这里**不用** `#[tokio::main]`：先同步地把命令
//! 定下来，走 GUI 就直接启动，其余命令再建 tokio runtime 执行。

use dmcl::cli::{self, Cli, Commands};
use dmcl::config::{Config, LoadedConfig};

fn main() {
    if let Err(err) = real_main() {
        eprintln!("错误：{err:#}");
        std::process::exit(1);
    }
}

fn real_main() -> anyhow::Result<()> {
    let parsed = Cli::parse_localized();
    // 启动时同步读一次配置：GUI 必须在建窗口 / GLib 主循环之前拿到它。
    let loaded = Config::load_sync(None);
    let root = match parsed.minecraft_root.as_deref() {
        Some(name) => loaded
            .config
            .root(name)
            .map_err(|err| anyhow::anyhow!("参数 --minecraft-root={name} 无效：{err:#}"))?,
        None => loaded.config.default_root()?,
    };

    match parsed.command {
        // 无子命令：有 GUI 前端就启动 GUI，没有就打印帮助并退出。
        None => dmcl::gui::run_default(loaded, root),
        // 显式 `gui`：前端不可用时直接报错，不回退。
        Some(Commands::Gui { frontend }) => dmcl::gui::run(frontend, loaded, root),
        Some(command) => {
            let LoadedConfig { mut config, error } = loaded;
            let runtime = tokio::runtime::Runtime::new()?;
            runtime.block_on(cli::dispatch(command, &mut config, error, &root))
        }
    }
}
