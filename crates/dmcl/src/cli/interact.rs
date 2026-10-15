//! 跟终端打交道的公用部件。
//!
//! 抽象出 [`Prompter`] 是为了让交互逻辑（拼参数、配置编辑、java 管理）能在没有
//! 终端的环境里被验证：真实实现是 [`TerminalPrompter`]（dialoguer），测试里换成
//! 脚本化回答。中文渲染见 [`ChineseTheme`]。

use std::fmt;

use anyhow::Result;
use dialoguer::console::Term;
use dialoguer::theme::Theme;
use dialoguer::{Confirm, Input, Select};

/// 跟终端打交道的方式。
pub(crate) trait Prompter {
    /// 问一个值参数。`Ok(None)` = 直接回车留空。
    ///
    /// `default` 会被**预填进输入框**（可以直接编辑，回车接受）。
    fn ask_value(&self, label: &str, default: &str) -> Result<Option<String>>;
    /// 问一个开关参数。
    fn ask_flag(&self, label: &str) -> Result<bool>;
    /// 从若干项里选一个。`Ok(None)` = 用户按 Esc 退出。
    fn choose(&self, prompt: &str, items: &[String]) -> Result<Option<usize>>;
}

/// 真实终端上的提问。
pub(crate) struct TerminalPrompter;

impl Prompter for TerminalPrompter {
    fn ask_value(&self, label: &str, default: &str) -> Result<Option<String>> {
        // **`allow_empty(true)` 不能省**：dialoguer 默认 `permit_empty = false`，
        // 而它"不允许空输入"的方式是直接 `continue` —— 既不报错也不给提示，
        // 只是把提问原样重新渲染一遍（dialoguer 0.11.0 `prompts/input.rs`）。
        // 用起来就是"必须输入某个值才能按回车"，且看不出任何原因。
        let mut input = Input::<String>::with_theme(&THEME)
            .allow_empty(true)
            // 默认值靠"预填进输入框"（initial text）呈现，不让 dialoguer 再渲染一遍
            .show_default(false)
            .with_prompt(label);
        if !default.is_empty() {
            input = input.with_initial_text(default);
        }
        // 首尾空白一并去掉：只敲了空格再回车同样算留空
        let value = input.interact_text()?.trim().to_string();
        Ok(if value.is_empty() { None } else { Some(value) })
    }

    fn ask_flag(&self, label: &str) -> Result<bool> {
        Ok(Confirm::with_theme(&THEME)
            .with_prompt(format!("是否{label}？"))
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

/// 把 dialoguer 自带的英文渲染换成中文。
///
/// dialoguer 的主题（`Theme`）就是一组 `format_*` 方法，只覆盖实际用到的几个，
/// 其余沿用 trait 的默认实现（如选择项的 `> ` 光标）。
pub(crate) struct ChineseTheme;

pub(crate) static THEME: ChineseTheme = ChineseTheme;

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

/// 一个可清屏的终端。
pub(crate) fn clear_screen() {
    let _ = Term::stdout().clear_screen();
}

/// 当前是否是可交互的终端（stdout 与 stderr 都得是）。
pub(crate) fn is_terminal() -> bool {
    // dialoguer 把提问写在 stderr 上、把菜单画面写在 stdout 上，两边都得是终端。
    Term::stdout().is_term() && Term::stderr().is_term()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 交互界面里的渲染不该残留 dialoguer 的英文。
    #[test]
    fn the_chinese_theme_has_no_english_left() {
        let render = |f: &dyn Fn(&mut dyn fmt::Write) -> fmt::Result| {
            let mut out = String::new();
            f(&mut out).unwrap();
            out
        };

        assert_eq!(
            render(&|f| THEME.format_input_prompt(f, "指定 Forge 版本", None)),
            "指定 Forge 版本："
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
}
