//! 交互式菜单（原 `dmcl.cli.prompt`）。
//!
//! 原实现自己写了一套终端界面：原生 `getch()`、手搓 ANSI 清屏、手拼 USAGE 字符串。
//! 这里由 [`dialoguer`] 提供菜单与输入（终端控制则用它再导出的 `console`），
//! 命令与参数元数据则由 `clap` 的内省给出 —— 一行手写界面代码都没有了。
//!
//! 行为保持不变：循环展示命令，回车选择，逐参数提问（显示默认值，开关用 `y/N`），
//! 然后通过启动器自己重新执行选中的命令。

use anyhow::Result;
use clap::CommandFactory;
use dialoguer::console::Term;
use dialoguer::{Confirm, Input, Select};

use dmcl_launch::escape_shell_command;

use super::Cli;

/// 全局参数，不值得逐个提问。
const SKIPPED_ARGS: &[&str] = &["help", "java", "mirror", "version"];

/// `clrscr()` —— 交给终端库，不再手写 ANSI 转义。
fn clear_screen() {
    let _ = Term::stdout().clear_screen();
}

/// `prompt` —— 交互式命令浏览器。
pub async fn prompt_interactive() -> Result<()> {
    loop {
        clear_screen();

        let root = Cli::command();
        let subcommands: Vec<clap::Command> = root
            .get_subcommands()
            .filter(|sub| sub.get_name() != "prompt" && sub.get_name() != "help")
            .cloned()
            .collect();
        if subcommands.is_empty() {
            println!("no commands");
            return Ok(());
        }

        let width = subcommands
            .iter()
            .map(|sub| sub.get_name().len())
            .max()
            .unwrap_or(0);
        let items: Vec<String> = subcommands
            .iter()
            .map(|sub| {
                format!(
                    "{:<width$}  {}",
                    sub.get_name(),
                    sub.get_about()
                        .map(|about| about.to_string())
                        .unwrap_or_default()
                )
            })
            .collect();

        let selection = Select::new()
            .with_prompt("Tip: W/S to control, Enter to select.")
            .items(&items)
            .default(0)
            .interact_opt()?;
        let Some(index) = selection else {
            // Esc ⇒ 退出菜单
            return Ok(());
        };

        let subcommand = subcommands[index].clone();
        let mut argv: Vec<String> = vec![subcommand.get_name().to_string()];

        for arg in subcommand.get_arguments() {
            let id = arg.get_id().as_str();
            if SKIPPED_ARGS.contains(&id) {
                continue;
            }
            let long = arg.get_long().map(str::to_string);

            if arg.get_action().takes_values() {
                let default = arg
                    .get_default_values()
                    .first()
                    .map(|value| value.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let label = if default.is_empty() {
                    id.to_string()
                } else {
                    format!("{id} (default: {default})")
                };
                let mut input = Input::<String>::new().with_prompt(label);
                if !default.is_empty() {
                    input = input.default(default);
                }
                let value = input.interact_text()?;
                match long {
                    Some(long) => argv.push(format!("--{long}={value}")),
                    None => argv.push(value),
                }
            } else {
                // 布尔开关
                let yes = Confirm::new()
                    .with_prompt(format!("{id}?"))
                    .default(false)
                    .interact()?;
                if yes {
                    if let Some(long) = long {
                        argv.push(format!("--{long}"));
                    }
                }
            }
        }

        clear_screen();
        println!("Running: {}", escape_shell_command(&argv));

        // 用拼好的参数重新执行自己，与原 `spawnProcess([thisExePath(), cmd.name, ...])` 一致
        let exe = std::env::current_exe()?;
        let _status = tokio::process::Command::new(exe)
            .args(&argv)
            .status()
            .await?;

        println!("\nPress enter to return.");
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
    }
}
