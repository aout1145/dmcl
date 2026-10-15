//! 各子命令的实现。
//!
//! `mod.rs`（父模块）只放命令定义、本地化与分发；这里按命令分文件，
//! 每个命令只关心自己那一件事。

pub(crate) mod config;
pub(crate) mod install;
pub(crate) mod java;
pub(crate) mod launch;
pub(crate) mod list;
pub(crate) mod version;

use dialoguer::Confirm;
use dmcl_task::TaskEvent;

use crate::cli::progress::ProgressPrinter;

/// 渲染一条任务事件；遇到**警告**就询问用户，让它决定继续还是中止。
pub(crate) fn handle_event(printer: &mut ProgressPrinter, event: TaskEvent) {
    let warning = match &event {
        TaskEvent::Warning {
            message, decision, ..
        } => Some((message.clone(), decision.clone())),
        _ => None,
    };
    printer.handle(event);

    if let Some((message, decision)) = warning {
        let proceed = if printer.is_interactive() {
            Confirm::new()
                .with_prompt(format!("⚠ {message}\n是否继续？"))
                .default(true)
                .interact()
                .unwrap_or(true)
        } else {
            true
        };
        if proceed {
            decision.accept();
        } else {
            decision.abort();
        }
    }
}
