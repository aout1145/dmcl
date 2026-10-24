//! `launch` —— 检查完整性、补全文件、生成参数、启动。

use std::sync::Arc;

use anyhow::Result;

use crate::config::{Config, MinecraftRoot};

use super::handle_event;
use crate::cli::progress::ProgressPrinter;

/// `dmcl launch <version-name>`。
pub(crate) async fn run(vername: &str, config: &Config, root: &MinecraftRoot) -> Result<()> {
    let loaded = config.load_instance(&root.path, vername).await;
    if let Some(error) = &loaded.error {
        for warning in &error.warnings {
            eprintln!("错误：{warning}");
        }
        return Err(error.clone().into());
    }
    let launch = loaded.launch;
    let option = config.launch_option(&launch, vername, &root.path)?;
    let install =
        Arc::new(config.install_options(config.download_source(), &launch, &root.path)?);

    let mut printer = ProgressPrinter::new();
    let mut monitor = dmcl_launch::launch_chain(option, install)
        .run_with((), |event| handle_event(&mut printer, event))
        .await?;
    printer.finish();

    monitor.wait().await?;
    Ok(())
}
