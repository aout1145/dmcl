//! Interactive prompt (was `dmcl.cli.prompt`).
//!
//! The original implemented its own terminal TUI: raw `getch()`, manual ANSI
//! screen clearing and hand-built USAGE strings. Here [`dialoguer`] provides
//! the menu / input widgets (and, through its re-exported [`console`] crate,
//! terminal control), while `clap`'s introspection provides the command and
//! parameter metadata — nothing is hand-rolled any more.
//!
//! Behaviour is preserved: a loop showing the commands, arrow/`W`/`S` + enter
//! selection, per-parameter prompting (defaults shown, `y/N` for flags), then
//! the selected command is re-executed through the launcher binary itself.

use anyhow::Result;
use clap::CommandFactory;
use dialoguer::console::Term;
use dialoguer::{Confirm, Input, Select};

use crate::utils::escape_shell_command;

use super::Cli;

/// Arguments that are global and therefore not worth prompting for.
const SKIPPED_ARGS: &[&str] = &["help", "java", "mirror", "version"];

/// `clrscr()` — delegated to the terminal crate instead of a literal ANSI
/// escape sequence.
fn clear_screen() {
    let _ = Term::stdout().clear_screen();
}

/// `prompt` — the interactive command browser.
pub async fn prompt_interactive() -> Result<()> {
    loop {
        clear_screen();

        let root = Cli::command();
        let subcommands: Vec<clap::Command> = root
            .get_subcommands()
            .filter(|s| s.get_name() != "prompt" && s.get_name() != "help")
            .cloned()
            .collect();
        if subcommands.is_empty() {
            println!("no commands");
            return Ok(());
        }

        let width = subcommands
            .iter()
            .map(|s| s.get_name().len())
            .max()
            .unwrap_or(0);
        let items: Vec<String> = subcommands
            .iter()
            .map(|s| {
                format!(
                    "{:<width$}  {}",
                    s.get_name(),
                    s.get_about().map(|a| a.to_string()).unwrap_or_default()
                )
            })
            .collect();

        let selection = Select::new()
            .with_prompt("Tip: W/S to control, Enter to select.")
            .items(&items)
            .default(0)
            .interact_opt()?;
        let Some(index) = selection else {
            // Esc ⇒ leave the prompt.
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
                    .map(|v| v.to_string_lossy().into_owned())
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
                // bool flag
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

        // Re-run ourselves with the assembled arguments, exactly like the
        // original `spawnProcess([thisExePath(), cmd.name, ...])`.
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
