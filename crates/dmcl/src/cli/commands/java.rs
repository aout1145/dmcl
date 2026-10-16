//! `java` —— 管理本机 java（自动搜索 / 手动添加 / 选择 / 删除）。
//!
//! 与终端无关的纯逻辑（合并 / 添加 / 选择 / 删除）在 [`crate::config::java`]；
//! 这里只负责命令分发与终端交互。

use std::path::PathBuf;

use anyhow::{anyhow, Result};

use dmcl_launch::{find_java, JavaChoice, JavaOption, DEFAULT_JAVA_ROOTS};

use crate::config::java::{insert_java, merge_java_configs, remove_java, set_java_choice};
use crate::config::Config;

use super::super::JavaCommand;
use crate::cli::interact::{clear_screen, is_terminal, Prompter, TerminalPrompter};

/// `dmcl java [子命令]`：没有子命令就进交互菜单。
pub(crate) async fn run(command: Option<JavaCommand>, config: &mut Config) -> Result<()> {
    match command {
        Some(command) => run_command(command, config).await,
        None => interactive(config).await,
    }
}

async fn run_command(command: JavaCommand, config: &mut Config) -> Result<()> {
    match command {
        JavaCommand::List => {
            list(config);
            Ok(())
        }
        JavaCommand::Search => search(config).await,
        JavaCommand::Add { path, name } => add(config, path, name).await,
        JavaCommand::Remove { name } => remove(config, &name).await,
        JavaCommand::Use { name } => use_choice(config, &name).await,
    }
}

/// `java list` —— 打印已配置的 java 与当前选择。
pub(crate) fn list(config: &Config) {
    if config.launch.java_configs.is_empty() {
        println!(
            "还没有配置任何 java。用 `dmcl java search` 自动搜索，或 `dmcl java add` 手动添加。"
        );
    } else {
        println!("已配置的 java：");
        for (name, path) in &config.launch.java_configs {
            println!("{name}: {}", path.display());
        }
    }
    println!("当前选择：{}", config.launch.java);
}

/// `java search` —— 扫描本机并**合并**进配置（保留手动添加的），不改当前选择。
pub(crate) async fn search(config: &mut Config) -> Result<()> {
    let found = find_java(DEFAULT_JAVA_ROOTS).await?;
    let added = merge_java_configs(config, &found);
    config.save().await?;
    println!("搜索到 {} 个 java，新加入 {added} 个。", found.len());
    list(config);
    Ok(())
}

/// `java add <path> [name]` —— 探测一个 java 并加入配置。
pub(crate) async fn add(config: &mut Config, path: PathBuf, name: Option<String>) -> Result<()> {
    let java = JavaOption::probe(path).await?;
    if !java.is_usable() {
        return Err(anyhow!("不是一个可用的 java：{}", java.path.display()));
    }
    let name = insert_java(config, &java, name.as_deref());
    config.save().await?;
    println!("已加入 {name}: {}", java.path.display());
    Ok(())
}

/// `java remove <name>` —— 删除一项；若正被选中则回退为 `autoselect`。
pub(crate) async fn remove(config: &mut Config, name: &str) -> Result<()> {
    let was_selected = remove_java(config, name)?;
    config.save().await?;
    if was_selected {
        println!("{name} 正被选中，已改回 autoselect。");
    }
    println!("已删除 {name}");
    Ok(())
}

/// `java use <name|autoselect>` —— 设置当前选择。
pub(crate) async fn use_choice(config: &mut Config, name: &str) -> Result<()> {
    set_java_choice(config, name)?;
    config.save().await?;
    println!("当前选择：{}", config.launch.java);
    Ok(())
}

// ---------------------------------------------------------------------------
// 交互菜单
// ---------------------------------------------------------------------------

/// 交互式 java 管理。非终端下退化成打印列表。
async fn interactive(config: &mut Config) -> Result<()> {
    if !is_terminal() {
        list(config);
        return Ok(());
    }
    let prompter = TerminalPrompter;
    loop {
        clear_screen();
        let items = vec![
            "列出已配置的 java".to_string(),
            "自动搜索本机 java".to_string(),
            "手动添加 java".to_string(),
            "选择要使用的 java".to_string(),
            "删除 java 配置".to_string(),
            "退出".to_string(),
        ];
        let Some(index) =
            prompter.choose("java 管理（↑/↓ 或 j/k 移动，回车确认，Esc 退出）", &items)?
        else {
            return Ok(());
        };
        match index {
            0 => {
                list(config);
                wait_enter();
            }
            1 => search(config).await?,
            2 => add_interactive(config, &prompter).await?,
            3 => use_interactive(config, &prompter).await?,
            4 => remove_interactive(config, &prompter).await?,
            _ => return Ok(()),
        }
        wait_enter();
    }
}

async fn add_interactive(config: &mut Config, prompter: &dyn Prompter) -> Result<()> {
    let Some(path) = prompter.ask_value("java 可执行文件路径", "")? else {
        println!("没有输入路径，已取消。");
        return Ok(());
    };
    let java = match JavaOption::probe(PathBuf::from(path)).await {
        Ok(java) if java.is_usable() => java,
        Ok(java) => {
            println!("不是可用的 java：{}", java.path.display());
            return Ok(());
        }
        Err(err) => {
            println!("探测失败：{err:#}");
            return Ok(());
        }
    };
    let name = prompter
        .ask_value("配置项名字", &java.config_name())?
        .unwrap_or_else(|| java.config_name());
    let name = insert_java(config, &java, Some(&name));
    config.save().await?;
    println!("已加入 {name}: {}", java.path.display());
    Ok(())
}

async fn use_interactive(config: &mut Config, prompter: &dyn Prompter) -> Result<()> {
    let mut items: Vec<String> = vec!["autoselect（自动选择）".to_string()];
    items.extend(config.launch.java_configs.keys().cloned());
    let Some(index) = prompter.choose("选择要使用的 java", &items)? else {
        return Ok(());
    };
    if index == 0 {
        config.launch.java = JavaChoice::Auto;
    } else {
        let name = config
            .launch
            .java_configs
            .keys()
            .nth(index - 1)
            .cloned()
            .unwrap_or_default();
        set_java_choice(config, &name)?;
    }
    config.save().await?;
    println!("当前选择：{}", config.launch.java);
    Ok(())
}

async fn remove_interactive(config: &mut Config, prompter: &dyn Prompter) -> Result<()> {
    let names: Vec<String> = config.launch.java_configs.keys().cloned().collect();
    if names.is_empty() {
        println!("还没有配置任何 java。");
        return Ok(());
    }
    let Some(index) = prompter.choose("删除哪个 java 配置", &names)? else {
        return Ok(());
    };
    remove(config, &names[index]).await
}

/// 等一次回车，方便看菜单动作的输出。
fn wait_enter() {
    println!("\n按回车继续。");
    let mut line = String::new();
    let _ = std::io::stdin().read_line(&mut line);
}
