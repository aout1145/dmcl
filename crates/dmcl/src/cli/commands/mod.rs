//! 各子命令的实现。
//!
//! `mod.rs`（父模块）只放命令定义、本地化与分发；这里按命令分文件，
//! 每个命令只关心自己那一件事。

pub(crate) mod install;
pub(crate) mod launch;
pub(crate) mod list;
pub(crate) mod search_java;
pub(crate) mod version;

use dialoguer::console::Term;
use dialoguer::Confirm;
use dmcl_task::TaskEvent;

use crate::cli::progress::ProgressPrinter;

/// 渲染一条任务事件；遇到**警告**就询问用户，让它决定继续还是中止。
pub(crate) fn handle_event(printer: &mut ProgressPrinter, event: TaskEvent) {
    let decision = match &event {
        TaskEvent::Warning { decision, .. } => Some(decision.clone()),
        _ => None,
    };
    printer.handle(event);

    if let Some(decision) = decision {
        let proceed = if printer.is_interactive() {
            // 警告正文已经由打印机打过一行（`⚠ 警告：…`），这里只问要不要继续
            let answer = Confirm::new()
                .with_prompt("是否继续？")
                .default(true)
                .interact()
                .unwrap_or(true);
            // dialoguer 选择后是 `write_line`（带换行），"是否继续？是" 留在**上一行**，
            // 清掉它，后续输出接着这一行写
            let _ = Term::stderr().clear_last_lines(1);
            answer
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
