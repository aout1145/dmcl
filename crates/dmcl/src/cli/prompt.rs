//! 交互式菜单。
//!
//! 由 [`dialoguer`] 提供菜单与输入（终端控制用它再导出的 `console`），
//! 命令与参数元数据则由 `clap` 的内省给出。
//!
//! # 结构
//!
//! 拆成三块，便于在没有终端的环境里验证：
//!
//! * [`Prompter`]：只管"跟终端打交道"；真实实现是 [`TerminalPrompter`]（dialoguer），
//!   测试里换成脚本化回答；
//! * [`build_argv`]：纯逻辑 —— 按 `clap` 的参数表逐个提问、拼出命令行，
//!   留空怎么处理、哪些参数不再追问都在这里，有单测；
//! * [`prompt_interactive`]：菜单主循环 + 用拼好的命令行重新执行自己。
//!
//! # 行为
//!
//! 行为：循环展示命令、回车选择、逐参数提问、再用启动器自己重新执行。几个体验细节：
//!
//! * **提示语全中文**：包括 dialoguer 自带的英文渲染（`[y/N]`、`error: …`），
//!   见 [`ChineseTheme`]；
//! * **可选参数可以留空**：值参数直接回车就是"不要这个参数"，
//!   不会再拼出一个空值（比如 `--forge-version=`）让 clap 报错；
//! * **必填参数留空会重新追问**，并给出中文说明（而不是沉默地重刷一次提问）。

use std::fmt;

use anyhow::Result;
use clap::CommandFactory;
use dialoguer::console::Term;
use dialoguer::theme::Theme;
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

// ---------------------------------------------------------------------------
// 提问：接口 + 真实实现
// ---------------------------------------------------------------------------

/// 跟终端打交道的方式。
///
/// 抽出来是为了让 [`build_argv`] 能在没有终端的环境里被验证：
/// 真实实现是 [`TerminalPrompter`]，测试里换成脚本化回答。
trait Prompter {
    /// 问一个值参数。`Ok(None)` = 直接回车留空。
    fn ask_value(&self, label: &str, default: &str) -> Result<Option<String>>;
    /// 问一个开关参数。
    fn ask_flag(&self, label: &str) -> Result<bool>;
    /// 选一个子命令。`Ok(None)` = 用户退出。
    fn choose(&self, prompt: &str, items: &[String]) -> Result<Option<usize>>;
}

/// 真实终端上的提问。
struct TerminalPrompter;

impl Prompter for TerminalPrompter {
    fn ask_value(&self, label: &str, default: &str) -> Result<Option<String>> {
        // **`allow_empty(true)` 不能省**：dialoguer 默认 `permit_empty = false`，
        // 而它"不允许空输入"的方式是直接 `continue` —— 既不报错也不给提示，
        // 只是把提问原样重新渲染一遍（dialoguer 0.11.0 `prompts/input.rs`）。
        // 用起来就是"必须输入某个值才能按回车"，且看不出任何原因。
        let mut input = Input::<String>::with_theme(&THEME)
            .allow_empty(true)
            // 标签里已经写了"（默认 …）"，不必让 dialoguer 再渲染一遍 `[值]: `
            .show_default(false)
            .with_prompt(label);
        if !default.is_empty() {
            input = input.default(default.to_string());
        }
        // 首尾空白一并去掉：只敲了空格再回车同样算留空
        let value = input.interact_text()?.trim().to_string();
        Ok(if value.is_empty() { None } else { Some(value) })
    }

    fn ask_flag(&self, label: &str) -> Result<bool> {
        Ok(Confirm::with_theme(&THEME)
            .with_prompt(format!("{label}？"))
            .default(false)
            .interact()?)
    }

    fn choose(&self, prompt: &str, items: &[String]) -> Result<Option<usize>> {
        Ok(Select::with_theme(&THEME)
            .with_prompt(prompt)
            .items(items)
            .default(0)
            .interact_opt()?)
    }
}

/// 逐个参数提问，拼出一条完整的命令行（第一个元素是子命令名）。
///
/// 纯逻辑，不碰终端：留空算跳过还是算无效输入、关掉的开关要不要继续追问，
/// 都在这一个函数里决定。
fn build_argv(subcommand: &clap::Command, prompter: &dyn Prompter) -> Result<Vec<String>> {
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
                match prompter.ask_value(&label, &default)? {
                    Some(value) => {
                        append_answer(&mut argv, long.as_deref(), true, &Answer::Value(value));
                        break;
                    }
                    // 留空：可选参数就是"不要这个参数"
                    None if !required => {
                        append_answer(&mut argv, long.as_deref(), true, &Answer::Skipped);
                        break;
                    }
                    None => {
                        attempts += 1;
                        if attempts >= MAX_REQUIRED_ATTEMPTS {
                            // 交给 clap 去报"缺少参数"，比在这里死循环友好
                            break;
                        }
                        println!("「{label}」是必填参数，不能留空。");
                    }
                }
            }
        } else {
            // 开关参数
            let label = arg
                .get_help()
                .map(|help| help.to_string())
                .unwrap_or_else(|| id.clone());
            if prompter.ask_flag(&label)? {
                append_answer(&mut argv, long.as_deref(), false, &Answer::Flag);
            } else {
                // 关掉的功能，它的细节参数不再问
                append_answer(&mut argv, long.as_deref(), false, &Answer::Skipped);
                disabled.push(id.clone());
            }
        }
    }

    Ok(argv)
}

// ---------------------------------------------------------------------------
// 渲染：中文主题
// ---------------------------------------------------------------------------

/// 把 dialoguer 自带的英文渲染换成中文。
///
/// dialoguer 的主题（`Theme`）就是一组 `format_*` 方法，只覆盖实际用到的几个，
/// 其余沿用 trait 的默认实现（如选择项的 `> ` 光标）。
struct ChineseTheme;

static THEME: ChineseTheme = ChineseTheme;

impl Theme for ChineseTheme {
    fn format_prompt(&self, f: &mut dyn fmt::Write, prompt: &str) -> fmt::Result {
        write!(f, "{prompt}：")
    }

    fn format_error(&self, f: &mut dyn fmt::Write, err: &str) -> fmt::Result {
        write!(f, "错误：{err}")
    }

    fn format_input_prompt(
        &self,
        f: &mut dyn fmt::Write,
        prompt: &str,
        default: Option<&str>,
    ) -> fmt::Result {
        match default {
            Some(default) => write!(f, "{prompt}（默认 {default}）："),
            None => write!(f, "{prompt}："),
        }
    }

    fn format_input_prompt_selection(
        &self,
        f: &mut dyn fmt::Write,
        prompt: &str,
        sel: &str,
    ) -> fmt::Result {
        write!(f, "{prompt}：{sel}")
    }

    fn format_confirm_prompt(
        &self,
        f: &mut dyn fmt::Write,
        prompt: &str,
        default: Option<bool>,
    ) -> fmt::Result {
        // y/n 是 dialoguer 唯一接受的按键，所以照实写出来，只是用中文解释
        match default {
            Some(true) => write!(f, "{prompt} [y=是 / n=否，回车=是] "),
            Some(false) => write!(f, "{prompt} [y=是 / n=否，回车=否] "),
            None => write!(f, "{prompt} [y=是 / n=否] "),
        }
    }

    fn format_confirm_prompt_selection(
        &self,
        f: &mut dyn fmt::Write,
        prompt: &str,
        selection: Option<bool>,
    ) -> fmt::Result {
        match selection {
            Some(true) => write!(f, "{prompt}是"),
            Some(false) => write!(f, "{prompt}否"),
            None => write!(f, "{prompt}"),
        }
    }
}

// ---------------------------------------------------------------------------
// 主循环
// ---------------------------------------------------------------------------

/// 清屏（交给终端库，不手写 ANSI 转义）。
fn clear_screen() {
    let _ = Term::stdout().clear_screen();
}

/// `prompt` —— 交互式命令浏览器。
pub async fn prompt_interactive() -> Result<()> {
    // 管道 / CI 里没有终端可以交互，直接给一句人话，而不是抛一个 IO 错误。
    // dialoguer 把提问写在 stderr 上、把菜单画面写在 stdout 上，两边都得是终端。
    if !Term::stdout().is_term() || !Term::stderr().is_term() {
        println!("当前不是交互式终端，无法使用交互式菜单。");
        println!("可以直接使用具体命令，例如：dmcl --help");
        return Ok(());
    }

    let prompter = TerminalPrompter;

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

        // 按键提示要与 dialoguer 实际支持的一致：上下键、j/k，回车确认，Esc 退出
        let selection = prompter.choose(
            "选择要执行的命令（↑/↓ 或 j/k 移动，回车确认，Esc 退出）",
            &items,
        )?;
        let Some(index) = selection else {
            // Esc ⇒ 退出菜单
            return Ok(());
        };

        let subcommand = subcommands[index].clone();
        let argv = build_argv(&subcommand, &prompter)?;

        clear_screen();
        println!("执行：{}", escape_shell_command(&argv));

        // 用拼好的参数重新执行自己
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
    use std::cell::RefCell;
    use std::collections::VecDeque;

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

    /// 交互界面里的渲染不该残留 dialoguer 的英文。
    #[test]
    fn the_chinese_theme_has_no_english_left() {
        let render = |f: &dyn Fn(&mut dyn fmt::Write) -> fmt::Result| {
            let mut out = String::new();
            f(&mut out).unwrap();
            out
        };

        assert_eq!(
            render(&|f| THEME.format_input_prompt(f, "指定 Forge 版本（可留空跳过）", None)),
            "指定 Forge 版本（可留空跳过）："
        );
        assert_eq!(
            render(&|f| THEME.format_input_prompt(f, "版本类型", Some("release"))),
            "版本类型（默认 release）："
        );
        assert_eq!(
            render(&|f| THEME.format_prompt(f, "选择要执行的命令")),
            "选择要执行的命令："
        );
        assert_eq!(
            render(&|f| THEME.format_error(f, "取值不合法")),
            "错误：取值不合法"
        );

        let confirm = render(&|f| THEME.format_confirm_prompt(f, "安装 Forge？", Some(false)));
        assert!(!confirm.contains("[y/N]"), "{confirm}");
        assert!(confirm.contains("回车=否"), "{confirm}");
        assert!(confirm.contains("y=是"), "{confirm}");

        assert_eq!(
            render(&|f| THEME.format_confirm_prompt_selection(f, "安装 Forge？", Some(true))),
            "安装 Forge？是"
        );
        assert_eq!(
            render(&|f| THEME.format_confirm_prompt_selection(f, "安装 Forge？", Some(false))),
            "安装 Forge？否"
        );
    }

    // -----------------------------------------------------------------------
    // 提问流程（用脚本化回答替代真实终端）
    // -----------------------------------------------------------------------

    /// 脚本化回答：按顺序给出预设答案，并记下实际问了哪些参数。
    struct Scripted {
        replies: RefCell<VecDeque<Reply>>,
        asked: RefCell<Vec<String>>,
    }

    #[derive(Debug, PartialEq, Eq)]
    enum Reply {
        /// 值回答；空字符串 = 直接回车留空。
        Value(&'static str),
        /// 开关回答。
        Flag(bool),
    }

    impl Scripted {
        fn new(replies: impl IntoIterator<Item = Reply>) -> Self {
            Self {
                replies: RefCell::new(replies.into_iter().collect()),
                asked: RefCell::new(Vec::new()),
            }
        }

        fn asked(&self) -> Vec<String> {
            self.asked.borrow().clone()
        }

        fn times_asked(&self, needle: &str) -> usize {
            self.asked().iter().filter(|q| q.contains(needle)).count()
        }
    }

    impl Prompter for Scripted {
        fn ask_value(&self, label: &str, _default: &str) -> Result<Option<String>> {
            self.asked.borrow_mut().push(label.to_string());
            match self.replies.borrow_mut().pop_front() {
                Some(Reply::Value("")) => Ok(None),
                Some(Reply::Value(value)) => Ok(Some(value.to_string())),
                other => panic!("期待一个值回答，实际拿到 {other:?}"),
            }
        }

        fn ask_flag(&self, label: &str) -> Result<bool> {
            self.asked.borrow_mut().push(label.to_string());
            match self.replies.borrow_mut().pop_front() {
                Some(Reply::Flag(yes)) => Ok(yes),
                other => panic!("期待一个开关回答，实际拿到 {other:?}"),
            }
        }

        fn choose(&self, _prompt: &str, _items: &[String]) -> Result<Option<usize>> {
            Ok(None)
        }
    }

    fn install_command() -> clap::Command {
        Cli::command()
            .find_subcommand("install")
            .expect("install 子命令")
            .clone()
    }

    /// 报告过的问题：可选参数不能留空。这里把整条提问流程跑一遍，
    /// 断言"直接回车"不会给命令行带上那个参数。
    #[test]
    fn an_optional_value_can_be_left_blank() {
        let prompter = Scripted::new([
            Reply::Value("1.20.1"), // mc 版本号
            Reply::Value("1.20.1"), // 本地版本目录名
            Reply::Flag(true),      // 顺便装 Forge
            Reply::Flag(false),     // 顺便装 NeoForge
            Reply::Flag(false),     // 顺便装 Fabric
            Reply::Value(""),       // Forge 版本：直接回车 ⇒ 跳过
        ]);

        let argv = build_argv(&install_command(), &prompter).unwrap();

        // 关键断言：没有 `--forge-version=`（更不是空值）
        assert_eq!(argv, vec!["install", "1.20.1", "1.20.1", "--forge"]);

        // 只问了 Forge 版本；被关掉的开关，其细节参数压根没问
        assert!(
            prompter.times_asked("指定 Forge 版本") == 1,
            "{:?}",
            prompter.asked()
        );
        assert_eq!(
            prompter.times_asked("NeoForge 版本"),
            0,
            "{:?}",
            prompter.asked()
        );
        assert_eq!(
            prompter.times_asked("Fabric loader 版本"),
            0,
            "{:?}",
            prompter.asked()
        );
        // 全局参数不问；必填参数各问一次（没被留空，不重问）
        assert_eq!(prompter.times_asked("mirror"), 0, "{:?}", prompter.asked());
        assert_eq!(
            prompter.times_asked("Minecraft 版本号"),
            1,
            "{:?}",
            prompter.asked()
        );
    }

    /// 必填参数留空要重新追问，而不是沉默地重刷提问。
    #[test]
    fn a_required_value_is_asked_again_after_being_left_blank() {
        let prompter = Scripted::new([
            Reply::Value(""),       // mc 版本号：留空 ⇒ 重问
            Reply::Value("1.20.1"), // 再填
            Reply::Value(""),       // 本地版本目录名：留空 ⇒ 重问
            Reply::Value("正式版"), // 再填
            Reply::Flag(false),
            Reply::Flag(false),
            Reply::Flag(false),
        ]);

        let argv = build_argv(&install_command(), &prompter).unwrap();

        assert_eq!(argv, vec!["install", "1.20.1", "正式版"]);
        assert_eq!(
            prompter.times_asked("Minecraft 版本号"),
            2,
            "{:?}",
            prompter.asked()
        );
        assert_eq!(
            prompter.times_asked("本地版本目录名"),
            2,
            "{:?}",
            prompter.asked()
        );
    }

    /// 必填参数一直留空时不会死循环：放宽给 clap 去报"缺少参数"。
    #[test]
    fn a_required_value_gives_up_after_a_few_blank_tries() {
        let prompter = Scripted::new([
            Reply::Value(""),
            Reply::Value(""),
            Reply::Value(""),
            Reply::Value(""),
            Reply::Value(""),
            Reply::Value(""),
            Reply::Flag(false),
            Reply::Flag(false),
            Reply::Flag(false),
        ]);

        let argv = build_argv(&install_command(), &prompter).unwrap();

        assert_eq!(argv, vec!["install"]);
        assert_eq!(
            prompter.times_asked("Minecraft 版本号"),
            MAX_REQUIRED_ATTEMPTS,
            "{:?}",
            prompter.asked()
        );
    }

    /// `--type` 这类"有默认值"的可选参数：回车不会拼出 `--type=` 空值。
    ///
    /// 真实终端下 dialoguer 会把回车解释成"接受默认值"（于是命令行里是
    /// `--type=release`）；逻辑层这里验证的是另一条路径 —— 空回答不产生参数，
    /// 由 clap 自己补默认值。两条路径的最终效果一致。
    #[test]
    fn an_optional_value_with_a_default_never_becomes_an_empty_value() {
        let list_mc = Cli::command()
            .find_subcommand("list-mc")
            .expect("list-mc 子命令")
            .clone();

        // 空回答 ⇒ 命令行里没有 `--type`（而不是 `--type=`）
        let prompter = Scripted::new([Reply::Value("")]);
        let argv = build_argv(&list_mc, &prompter).unwrap();
        assert_eq!(argv, vec!["list-mc"]);

        // 而且 clap 侧确实有默认值，所以命令仍然按 release 过滤
        assert!(list_mc
            .get_arguments()
            .find(|arg| arg.get_id() == "type")
            .is_some_and(|arg| !arg.get_default_values().is_empty()));
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
