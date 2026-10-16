//! 交互式菜单。
//!
//! 由 [`dialoguer`] 提供菜单与输入（终端控制用它再导出的 `console`），
//! 命令与参数元数据则由 `clap` 的内省给出。
//!
//! # 结构
//!
//! 拆成三块，便于在没有终端的环境里验证：
//!
//! * [`Prompter`]（见 [`crate::cli::interact`]）：只管"跟终端打交道"；
//!   真实实现是 `TerminalPrompter`（dialoguer），测试里换成脚本化回答；
//! * [`build_argv`]：异步纯逻辑 —— 按 `clap` 的参数表逐个提问、拼出命令行，
//!   留空怎么处理、哪些参数不再追问、加载器版本的默认值都在这里，有单测；
//! * [`prompt_interactive`]：菜单主循环 + 用拼好的命令行重新执行自己。
//!
//! # 行为
//!
//! * **提示语全中文**：包括 dialoguer 自带的英文渲染（`[y/N]`、`error: …`）；
//! * **默认值预填进输入框**：值参数有默认值时直接填在输入框里，回车即接受（不再在
//!   标签里写「（默认 …）」）；加载器版本（`--forge-version` 等）会按已经填好的 mc
//!   版本**现场查最新**再预填；
//! * **可选参数可以留空**：值参数清空后回车就是"不要这个参数"，
//!   不会再拼出一个空值（比如 `--forge-version=`）让 clap 报错；
//! * **必填参数留空会重新追问**，并给出中文说明（而不是沉默地重刷一次提问）。

use std::collections::BTreeMap;

use anyhow::Result;
use clap::CommandFactory;

use dmcl_task::BoxFuture;

use super::interact::{clear_screen, is_terminal, Prompter, TerminalPrompter};
use super::Cli;

/// 交互式拼命令行时，为参数求「预填默认值」的接口。
///
/// prompt 只依赖这个接口，**不认识任何具体命令的参数**；各命令按需实现它 ——
/// 例如 `install` 会现场查加载器最新版本、按已选项拼默认版本名。
pub(crate) trait ArgDefaults {
    /// `id` 是参数 id，`answers` 是已经填过的值（参数 id → 值），
    /// `enabled_flags` 是已经启用的开关 id。
    ///
    /// 返回 `None` 表示「没有默认值」，由调用方退回 clap 的静态默认值 / 留空。
    fn default_for<'a>(
        &'a self,
        id: &'a str,
        answers: &'a BTreeMap<String, String>,
        enabled_flags: &'a [String],
    ) -> BoxFuture<'a, Option<String>>;
}

/// 全局参数，不值得逐个提问。
const SKIPPED_ARGS: &[&str] = &["help", "version"];

/// 不进交互菜单的命令：`prompt` 不能递归自己，`help` 是 clap 内置的，
/// `gui` 必须在主线程启动（此时进程已经跑在 tokio 里，dispatcher 也会拒绝）。
const HIDDEN_COMMANDS: &[&str] = &["prompt", "help", "gui"];

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
// 拼命令行
// ---------------------------------------------------------------------------

/// 逐个参数提问，拼出一条完整的命令行（第一个元素是子命令名）。
///
/// 纯逻辑（除了 [`ArgDefaults`] 可能联网查默认值），不碰终端：留空算跳过还是算
/// 无效输入、关掉的开关要不要继续追问、默认值怎么来，都在这一个函数里决定。
async fn build_argv(
    subcommand: &clap::Command,
    prompter: &dyn Prompter,
    defaults: &dyn ArgDefaults,
) -> Result<Vec<String>> {
    let mut argv: Vec<String> = vec![subcommand.get_name().to_string()];
    // 被关掉的开关；它的细节参数之后不再追问
    let mut disabled: Vec<String> = Vec::new();
    // 已经填过的值，按参数 id 记下来（加载器版本要看 mc 版本、版本名要看加载器）
    let mut answers: BTreeMap<String, String> = BTreeMap::new();
    // 已经启用的开关 id，给版本名的默认值用
    let mut enabled_flags: Vec<String> = Vec::new();

    for arg in subcommand.get_arguments() {
        let id = arg.get_id().as_str().to_string();
        if SKIPPED_ARGS.contains(&id.as_str()) || !should_ask(&id, &disabled) {
            continue;
        }
        let long: Option<String> = arg.get_long().map(str::to_string);

        if arg.get_action().takes_values() {
            let required = arg.is_required_set();
            // 先看 clap 的静态默认值，再由命令提供的默认值覆盖（可能要联网查）
            let mut default = arg
                .get_default_values()
                .first()
                .map(|value| value.to_string_lossy().into_owned())
                .unwrap_or_default();
            if let Some(computed) = defaults.default_for(&id, &answers, &enabled_flags).await {
                default = computed;
            }

            let label = arg
                .get_help()
                .map(|help| help.to_string())
                .unwrap_or_else(|| id.clone());

            let mut attempts = 0;
            loop {
                match prompter.ask_value(&label, &default)? {
                    Some(value) => {
                        answers.insert(id.clone(), value.clone());
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
                enabled_flags.push(id.clone());
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
// 主循环
// ---------------------------------------------------------------------------

/// 交互菜单里要显示的命令（去掉 [`HIDDEN_COMMANDS`]）。
fn menu_commands() -> Vec<clap::Command> {
    Cli::command()
        .get_subcommands()
        .filter(|sub| !HIDDEN_COMMANDS.contains(&sub.get_name()))
        .cloned()
        .collect()
}

/// `prompt` —— 交互式命令浏览器。`defaults` 为各命令提供参数默认值。
pub(crate) async fn prompt_interactive(defaults: &dyn ArgDefaults) -> Result<()> {
    // 管道 / CI 里没有终端可以交互，直接给一句人话，而不是抛一个 IO 错误。
    // dialoguer 把提问写在 stderr 上、把菜单画面写在 stdout 上，两边都得是终端。
    if !is_terminal() {
        println!("当前不是交互式终端，无法使用交互式菜单。");
        println!("可以直接使用具体命令，例如：dmcl --help");
        return Ok(());
    }

    let prompter = TerminalPrompter;

    loop {
        clear_screen();

        let subcommands = menu_commands();
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
        let argv = build_argv(&subcommand, &prompter, defaults).await?;

        clear_screen();
        println!("执行：{}", shell_words::join(&argv));

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
    fn a_disabled_flag_hides_its_detail_arguments() {
        let disabled = vec!["forge".to_string()];
        // 没装 forge 就不该再问 forge 版本
        assert!(!should_ask("forge_version", &disabled));
        assert!(should_ask("fabric_version", &disabled));
        assert!(should_ask("version_id", &disabled));
        // 什么都没关时全都要问
        assert!(should_ask("forge_version", &[]));
    }

    /// 交互菜单不列出 `prompt` / `help` / `gui`。
    #[test]
    fn the_prompt_menu_hides_prompt_help_and_gui() {
        let names: Vec<String> = menu_commands()
            .iter()
            .map(|sub| sub.get_name().to_string())
            .collect();
        for hidden in HIDDEN_COMMANDS {
            assert!(
                !names.contains(&hidden.to_string()),
                "菜单里不该有 {hidden}: {names:?}"
            );
        }
        assert!(names.contains(&"launch".to_string()), "{names:?}");
    }

    // -----------------------------------------------------------------------
    // 提问流程（用脚本化回答替代真实终端）
    // -----------------------------------------------------------------------

    /// 脚本化回答：按顺序给出预设答案，并记下实际问了哪些参数（含传来的默认值）。
    struct Scripted {
        replies: RefCell<VecDeque<Reply>>,
        asked: RefCell<Vec<String>>,
        defaults: RefCell<Vec<(String, String)>>,
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
                defaults: RefCell::new(Vec::new()),
            }
        }

        fn asked(&self) -> Vec<String> {
            self.asked.borrow().clone()
        }

        fn times_asked(&self, needle: &str) -> usize {
            self.asked().iter().filter(|q| q.as_str() == needle).count()
        }

        /// 某个问题收到的默认值（用于断言"预填了默认名 / 最新版本"）。
        fn default_of(&self, needle: &str) -> Option<String> {
            self.defaults
                .borrow()
                .iter()
                .find(|(label, _)| label == needle)
                .map(|(_, default)| default.clone())
        }
    }

    impl Prompter for Scripted {
        fn ask_value(&self, label: &str, default: &str) -> Result<Option<String>> {
            self.asked.borrow_mut().push(label.to_string());
            self.defaults
                .borrow_mut()
                .push((label.to_string(), default.to_string()));
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

    /// 假的默认值提供者：给每个参数一个可预测的默认值，验证 prompt 会预填。
    struct FakeDefaults;

    impl ArgDefaults for FakeDefaults {
        fn default_for<'a>(
            &'a self,
            id: &'a str,
            _answers: &'a BTreeMap<String, String>,
            _enabled_flags: &'a [String],
        ) -> BoxFuture<'a, Option<String>> {
            Box::pin(async move { Some(format!("default-{id}")) })
        }
    }

    fn install_command() -> clap::Command {
        Cli::command()
            .find_subcommand("install")
            .expect("install 子命令")
            .clone()
    }

    fn list_forge_command() -> clap::Command {
        Cli::command()
            .find_subcommand("list-forge")
            .expect("list-forge 子命令")
            .clone()
    }

    /// 报告过的问题：可选参数不能留空。这里把整条提问流程跑一遍，
    /// 断言"直接回车"不会给命令行带上那个参数。
    #[tokio::test]
    async fn an_optional_value_can_be_left_blank() {
        let prompter = Scripted::new([
            Reply::Value("1.20.1"), // mc 版本号
            Reply::Flag(true),      // 顺便装 Forge
            Reply::Flag(false),     // 顺便装 NeoForge
            Reply::Flag(false),     // 顺便装 Fabric
            Reply::Value(""),       // Forge 版本：直接回车 ⇒ 跳过
            Reply::Value(""),       // 本地版本目录名：直接回车 ⇒ 跳过
        ]);

        let argv = build_argv(&install_command(), &prompter, &FakeDefaults)
            .await
            .unwrap();

        // 关键断言：没有 `--forge-version=`（更不是空值）
        assert_eq!(argv, vec!["install", "1.20.1", "--forge"]);

        // 只问了 Forge 版本；被关掉的开关，其细节参数压根没问
        assert!(
            prompter.times_asked("Forge 版本") == 1,
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
        assert_eq!(prompter.times_asked("mirror"), 0, "{:?}", prompter.asked());
        assert_eq!(
            prompter.times_asked("Minecraft 版本号"),
            1,
            "{:?}",
            prompter.asked()
        );
        // 版本名的默认值来自提供者（具体拼法在 install 的测试里验证）
        assert_eq!(
            prompter.default_of("本地版本名").as_deref(),
            Some("default-version_name")
        );
    }

    /// 各参数的默认值由 [`ArgDefaults`] 提供者预填；prompt 自己不认任何具体命令。
    #[tokio::test]
    async fn defaults_come_from_the_provider() {
        let prompter = Scripted::new([
            Reply::Value("1.20.1"),              // mc 版本号
            Reply::Flag(true),                   // 顺便装 Forge
            Reply::Flag(false),                  // 顺便装 NeoForge
            Reply::Flag(false),                  // 顺便装 Fabric
            Reply::Value("47.2.0"),              // Forge 版本（真实终端里回车即接受预填默认值）
            Reply::Value("1.20.1-forge-47.2.0"), // 版本名（默认名）
        ]);

        let argv = build_argv(&install_command(), &prompter, &FakeDefaults)
            .await
            .unwrap();

        // 加载器版本与版本名的默认值都来自提供者
        assert_eq!(
            prompter.default_of("Forge 版本").as_deref(),
            Some("default-forge_version")
        );
        assert_eq!(
            prompter.default_of("本地版本名").as_deref(),
            Some("default-version_name")
        );
        assert_eq!(
            argv,
            vec![
                "install",
                "1.20.1",
                "--forge",
                "--forge-version=47.2.0",
                "1.20.1-forge-47.2.0",
            ]
        );
    }

    /// 必填参数留空要重新追问，而不是沉默地重刷提问。
    #[tokio::test]
    async fn a_required_value_is_asked_again_after_being_left_blank() {
        let prompter = Scripted::new([
            Reply::Value(""),       // mc 版本号：留空 ⇒ 重问
            Reply::Value("1.20.1"), // 再填
        ]);

        let argv = build_argv(&list_forge_command(), &prompter, &FakeDefaults)
            .await
            .unwrap();

        assert_eq!(argv, vec!["list-forge", "1.20.1"]);
        assert_eq!(
            prompter.times_asked("Minecraft 版本号"),
            2,
            "{:?}",
            prompter.asked()
        );
    }

    /// 必填参数一直留空时不会死循环：放宽给 clap 去报"缺少参数"。
    #[tokio::test]
    async fn a_required_value_gives_up_after_a_few_blank_tries() {
        let prompter = Scripted::new([Reply::Value(""), Reply::Value(""), Reply::Value("")]);

        let argv = build_argv(&list_forge_command(), &prompter, &FakeDefaults)
            .await
            .unwrap();

        assert_eq!(argv, vec!["list-forge"]);
        assert_eq!(
            prompter.times_asked("Minecraft 版本号"),
            MAX_REQUIRED_ATTEMPTS,
            "{:?}",
            prompter.asked()
        );
    }

    /// `--type` 这类"有默认值"的可选参数：回车不会拼出 `--type=` 空值。
    #[tokio::test]
    async fn an_optional_value_with_a_default_never_becomes_an_empty_value() {
        let list_mc = Cli::command()
            .find_subcommand("list-mc")
            .expect("list-mc 子命令")
            .clone();

        // 空回答 ⇒ 命令行里没有 `--type`（而不是 `--type=`）
        let prompter = Scripted::new([Reply::Value("")]);
        let argv = build_argv(&list_mc, &prompter, &FakeDefaults)
            .await
            .unwrap();
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

        // 位置参数：mc 版本号必填，版本目录名可选
        let version_id = install
            .get_arguments()
            .find(|arg| arg.get_id() == "version_id")
            .expect("version_id");
        assert!(version_id.is_required_set());
        let version_name = install
            .get_arguments()
            .find(|arg| arg.get_id() == "version_name")
            .expect("version_name");
        assert!(!version_name.is_required_set());

        // `--forge-version` 这类是可选的（可以直接留空跳过）
        let forge_version = install
            .get_arguments()
            .find(|arg| arg.get_id() == "forge_version")
            .expect("forge_version");
        assert!(!forge_version.is_required_set());
        assert_eq!(forge_version.get_long(), Some("forge-version"));
    }
}
