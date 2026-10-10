//! 交互式菜单（原 `dmcl.cli.prompt`）。
//!
//! 原实现自己写了一套终端界面：原生 `getch()`、手搓 ANSI 清屏、手拼 USAGE 字符串。
//! 这里由 [`dialoguer`] 提供菜单与输入（终端控制则用它再导出的 `console`），
//! 命令与参数元数据则由 `clap` 的内省给出 —— 一行手写界面代码都没有了。
//!
//! 行为与原实现一致（循环展示命令、回车选择、逐参数提问、通过启动器自己重新执行），
//! 另外修了两个体验问题：
//!
//! * **提示语全中文**，并且直接用命令自己的中文说明（`clap` 里的 `help`）
//!   作为标签，而不是把裸的参数 id 丢给用户；
//! * **可选参数可以留空**：值参数直接回车就是"不要这个参数"，
//!   不会再拼出一个空值（比如 `--forge-version=`）让 clap 报错；
//!   必填参数留空则会重新追问。

use anyhow::Result;
use clap::CommandFactory;
use dialoguer::console::Term;
use dialoguer::{Confirm, Input, Select};

use dmcl_launch::escape_shell_command;

use super::Cli;

/// 全局参数，不值得逐个提问。
const SKIPPED_ARGS: &[&str] = &["help", "java", "mirror", "version"];

/// 必填参数被反复留空时最多追问几次（之后交给 clap 报错，避免死循环）。
const MAX_REQUIRED_ATTEMPTS: usize = 3;

/// 一次提问的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
enum Answer {
    /// 填了一个值。
    Value(String),
    /// 开关：启用。
    Flag,
    /// 留空 / 选择不启用 —— 命令行里**什么都不加**。
    Skipped,
}

/// 把一次回答写进 argv。
///
/// "可选选项能留空"就是靠这里：`Answer::Skipped` 不产生任何参数。
fn append_answer(argv: &mut Vec<String>, long: Option<&str>, takes_value: bool, answer: &Answer) {
    match (takes_value, answer) {
        (true, Answer::Value(value)) => match long {
            Some(long) => argv.push(format!("--{long}={value}")),
            None => argv.push(value.clone()),
        },
        (false, Answer::Flag) => {
            if let Some(long) = long {
                argv.push(format!("--{long}"));
            }
        }
        // 跳过：什么都不加
        _ => {}
    }
}

/// 值参数的提问标签：优先用命令里的中文说明，并提示"默认值"或"可留空"。
fn value_label(arg: &clap::Arg, default: &str, required: bool) -> String {
    let name = arg
        .get_help()
        .map(|help| help.to_string())
        .unwrap_or_else(|| arg.get_id().to_string());
    if !default.is_empty() {
        format!("{name}（默认 {default}）")
    } else if required {
        name
    } else {
        format!("{name}（可留空跳过）")
    }
}

/// 这个参数还要不要问。
///
/// 关掉的开关，它的细节参数就不再追问了：`--forge` 选了"否"，
/// 后面那个 `--forge-version` 就不该再出现。
fn should_ask(id: &str, disabled: &[String]) -> bool {
    !disabled
        .iter()
        .any(|prefix| id.starts_with(prefix.as_str()))
}

/// `clrscr()` —— 交给终端库，不再手写 ANSI 转义。
fn clear_screen() {
    let _ = Term::stdout().clear_screen();
}

/// `prompt` —— 交互式命令浏览器。
pub async fn prompt_interactive() -> Result<()> {
    // 管道 / CI 里没有终端可以交互，直接给一句人话，而不是抛一个 IO 错误
    if !Term::stdout().is_term() {
        println!("当前不是交互式终端，无法使用交互式菜单。");
        println!("可以直接使用具体命令，例如：dmcl --help");
        return Ok(());
    }

    loop {
        clear_screen();

        let root = Cli::command();
        let subcommands: Vec<clap::Command> = root
            .get_subcommands()
            .filter(|sub| sub.get_name() != "prompt" && sub.get_name() != "help")
            .cloned()
            .collect();
        if subcommands.is_empty() {
            println!("没有可用的命令");
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
            .with_prompt("提示：W/S 选择，回车确认（Esc 退出）")
            .items(&items)
            .default(0)
            .interact_opt()?;
        let Some(index) = selection else {
            // Esc ⇒ 退出菜单
            return Ok(());
        };

        let subcommand = subcommands[index].clone();
        let mut argv: Vec<String> = vec![subcommand.get_name().to_string()];
        // 被关掉的开关；它的细节参数之后不再追问
        let mut disabled: Vec<String> = Vec::new();

        for arg in subcommand.get_arguments() {
            let id = arg.get_id().as_str().to_string();
            if SKIPPED_ARGS.contains(&id.as_str()) || !should_ask(&id, &disabled) {
                continue;
            }
            let long: Option<String> = arg.get_long().map(str::to_string);

            if arg.get_action().takes_values() {
                let required = arg.is_required_set();
                let default = arg
                    .get_default_values()
                    .first()
                    .map(|value| value.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let label = value_label(arg, &default, required);

                let mut attempts = 0;
                loop {
                    let mut input = Input::<String>::new().with_prompt(label.clone());
                    if !default.is_empty() {
                        input = input.default(default.clone());
                    }
                    let value = input.interact_text()?;

                    if !value.is_empty() {
                        append_answer(&mut argv, long.as_deref(), true, &Answer::Value(value));
                        break;
                    }
                    // 留空：可选参数就是"不要这个参数"
                    if !required {
                        break;
                    }
                    attempts += 1;
                    if attempts >= MAX_REQUIRED_ATTEMPTS {
                        // 交给 clap 去报"缺少参数"，比在这里死循环友好
                        break;
                    }
                    println!("「{label}」是必填参数，不能留空。");
                }
            } else {
                // 开关参数
                let label = arg
                    .get_help()
                    .map(|help| help.to_string())
                    .unwrap_or_else(|| id.clone());
                let yes = Confirm::new()
                    .with_prompt(format!("{label}？"))
                    .default(false)
                    .interact()?;
                if yes {
                    append_answer(&mut argv, long.as_deref(), false, &Answer::Flag);
                } else {
                    // 关掉的功能，它的细节参数不再问
                    append_answer(&mut argv, long.as_deref(), false, &Answer::Skipped);
                    disabled.push(id.clone());
                }
            }
        }

        clear_screen();
        println!("执行：{}", escape_shell_command(&argv));

        // 用拼好的参数重新执行自己，与原 `spawnProcess([thisExePath(), cmd.name, ...])` 一致
        let exe = std::env::current_exe()?;
        let _status = tokio::process::Command::new(exe)
            .args(&argv)
            .status()
            .await?;

        println!("\n按回车返回。");
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skipped_answers_add_nothing_to_the_command_line() {
        let mut argv = vec!["install".to_string()];
        append_answer(&mut argv, Some("forge-version"), true, &Answer::Skipped);
        append_answer(&mut argv, Some("forge"), false, &Answer::Skipped);
        // 留空的可选参数不会拼出 `--forge-version=` 这种空值
        assert_eq!(argv, vec!["install"]);
    }

    #[test]
    fn values_and_flags_are_rendered_like_the_original_prompt() {
        let mut argv = vec!["install".to_string()];
        append_answer(&mut argv, None, true, &Answer::Value("1.20.1".into()));
        append_answer(
            &mut argv,
            Some("forge-version"),
            true,
            &Answer::Value("47.2.0".into()),
        );
        append_answer(&mut argv, Some("forge"), false, &Answer::Flag);
        append_answer(&mut argv, Some("fabric"), false, &Answer::Skipped);
        assert_eq!(
            argv,
            vec!["install", "1.20.1", "--forge-version=47.2.0", "--forge"]
        );
    }

    #[test]
    fn labels_use_the_chinese_help_text_and_announce_skipping() {
        let optional = clap::Arg::new("forge_version")
            .long("forge-version")
            .help("指定 Forge 版本");
        assert_eq!(
            value_label(&optional, "", false),
            "指定 Forge 版本（可留空跳过）"
        );
        assert_eq!(
            value_label(&optional, "47.2.0", false),
            "指定 Forge 版本（默认 47.2.0）"
        );

        // 必填参数不提示"可留空"
        let required = clap::Arg::new("version_id")
            .help("Minecraft 版本号")
            .required(true);
        assert_eq!(value_label(&required, "", true), "Minecraft 版本号");

        // 没有 help 时退回参数 id
        let bare = clap::Arg::new("bare");
        assert_eq!(value_label(&bare, "", true), "bare");
    }

    #[test]
    fn a_disabled_flag_hides_its_detail_arguments() {
        let disabled = vec!["forge".to_string()];
        // 没装 forge 就不该再问 forge 版本
        assert!(!should_ask("forge_version", &disabled));
        assert!(should_ask("fabric_version", &disabled));
        assert!(should_ask("version_id", &disabled));
        // 什么都没关时全都要问
        assert!(should_ask("forge_version", &[]));
    }

    #[test]
    fn the_real_command_line_marks_optional_arguments_as_optional() {
        let root = Cli::command();
        let install = root.find_subcommand("install").expect("install 子命令");

        // 位置参数必填
        let version_id = install
            .get_arguments()
            .find(|arg| arg.get_id() == "version_id")
            .expect("version_id");
        assert!(version_id.is_required_set());

        // `--forge-version` 这类是可选的（可以直接留空跳过）
        let forge_version = install
            .get_arguments()
            .find(|arg| arg.get_id() == "forge_version")
            .expect("forge_version");
        assert!(!forge_version.is_required_set());
        assert_eq!(forge_version.get_long(), Some("forge-version"));

        // 被跳过的全局参数确实存在（`help` 由 clap 自动生成，不在 get_arguments 里）
        for skipped in ["java", "mirror"] {
            assert!(
                root.get_arguments().any(|arg| arg.get_id() == skipped),
                "{skipped} 应当确实存在"
            );
        }
    }
}
