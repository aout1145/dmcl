//! `launch` —— 检查完整性、补全文件、生成参数、启动。

use std::sync::Arc;

use anyhow::Result;

use crate::config::Config;

use super::handle_event;
use crate::cli::progress::ProgressPrinter;

/// `dmcl launch <version-name>`。
pub(crate) async fn run(vername: &str, config: &Config) -> Result<()> {
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
