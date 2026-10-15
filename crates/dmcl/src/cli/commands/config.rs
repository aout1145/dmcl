//! `config` —— 交互式配置编辑器（也能打印）。
//!
//! 配置项清单由 [`Config::fields`] 从 serde 派生自动展开，不手写；标量值单行编辑
//! （预填当前值）。结构化字段（对象 / 数组，如 `launch.java_configs`）**不支持**在
//! 这里修改，选中时只提示一句。非终端下退化成只打印。

use anyhow::Result;

use crate::cli::interact::{clear_screen, is_terminal, Prompter, TerminalPrompter};
use crate::config::{display_value, is_json_value, parse_edited_value, Config};

/// `dmcl config`。
pub(crate) async fn run(config: &mut Config) -> Result<()> {
    if !is_terminal() {
        config.show();
        return Ok(());
    }
    edit_loop(config, &TerminalPrompter).await
}

/// 菜单循环：选一项 → 编辑 → 写回 → 重新渲染。
async fn edit_loop(config: &mut Config, prompter: &dyn Prompter) -> Result<()> {
    loop {
        clear_screen();
        let fields = config.fields()?;
        let items: Vec<String> = fields
            .iter()
            .map(|(key, value)| format!("{key} = {}", display_value(value)))
            .collect();
        let prompt = format!(
            "配置文件：{}\n选择要修改的配置项（↑/↓ 或 j/k 移动，回车编辑，Esc 退出）",
            Config::path().display()
        );

        let Some(index) = prompter.choose(&prompt, &items)? else {
            return Ok(());
        };
        let (key, current) = fields[index].clone();

        // 结构化字段不在交互编辑器里改
        if is_json_value(&current) {
            println!("「{key}」不能在这里修改。");
            wait_enter();
            continue;
        }

        // 标量：单行输入，预填当前值（回车即保留 / 写入）
        let edited = prompter
            .ask_value(&key, &display_value(&current))?
            .unwrap_or_default();

        let value = match parse_edited_value(&edited, &current) {
            Ok(value) => value,
            Err(err) => {
                println!("没有修改 {key}：{err:#}");
                wait_enter();
                continue;
            }
        };
        match config.set_field(&key, value) {
            Ok(()) => config.save().await?,
            Err(err) => {
                println!("没有修改 {key}：{err:#}");
                wait_enter();
            }
        }
    }
}

fn wait_enter() {
    println!("\n按回车继续。");
    let mut line = String::new();
    let _ = std::io::stdin().read_line(&mut line);
}
