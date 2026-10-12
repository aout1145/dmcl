//! 把任务事件流渲染成终端进度。

use std::io::Write;

use dialoguer::console::Term;
use dmcl_task::TaskEvent;

/// 一层内容的缩进（与 `[i/n] ` 等宽）。
const INDENT: &str = "      ";
/// 每再深一层的额外缩进。
const NESTED: &str = "  ";

/// 进度打印机。
///
/// 渲染成「步骤头 + 缩进内容」，嵌套链层层缩进：
///
/// ```text
/// 安装 1.20.1（共 4 步）
/// [1/4] 获取原版元数据
///       查询版本列表并保存 1.20.1.json
///       完成
/// [4/4] 下载游戏文件
///       获取资源索引
///         100.0% (403.4 KiB / 403.4 KiB) 1/1 个文件
///       下载库文件
///         41.2% (34.9 MiB / 84.7 MiB) 35/120 个文件
///       完成
/// 安装 1.20.1 完成
/// ```
///
/// 层级由 [`Layout`] 用**栈**还原：嵌套链的 `StepStarted` 入栈、`StepFinished` 出栈，
/// 栈深即层级，内容按层级缩进，所以嵌套步骤名只作标题出现一次、不再逐行重复。
///
/// 事件分两类：**进度**（`TaskEvent::Progress`）是"可原地刷新"的行——交互式终端上
/// 反复覆盖同一行，管道里跨 5% 刻度才打一行；**日志**（`TaskEvent::Log`）是"最终行"
/// ——逐条输出、不覆盖、不去重（命令行输出、缺件清单等）。
pub struct ProgressPrinter {
    term: Term,
    interactive: bool,
    /// 非交互模式：上一个进度百分比刻度。
    last_bucket: Option<u64>,
    /// 非交互模式：上一行文字（去重用）。
    last_line: String,
    /// 层级状态（栈 + 懒标签）。
    layout: Layout,
    /// 交互模式：当前悬着那一行的前缀（判断要不要先换行）。
    live_subject: String,
    /// 交互模式：当前行是否"悬着"（还没换行，等着被覆盖 / 收尾）。
    mid_line: bool,
}

impl Default for ProgressPrinter {
    fn default() -> Self {
        Self::new()
    }
}

impl ProgressPrinter {
    pub fn new() -> Self {
        let term = Term::stdout();
        let interactive = term.is_term();
        Self {
            term,
            interactive,
            last_bucket: None,
            last_line: String::new(),
            layout: Layout::default(),
            live_subject: String::new(),
            mid_line: false,
        }
    }

    /// 直接交给 `TaskChain::run_with`。
    pub fn handle(&mut self, event: TaskEvent) {
        if matches!(event, TaskEvent::StepStarted { .. }) {
            self.last_bucket = None;
        }

        // 非交互模式的节流：字节进度只在跨 5% 刻度时打
        if !self.interactive {
            if let TaskEvent::Progress { progress, .. } = &event {
                let bucket = progress.percent().map(|percent| (percent / 5.0) as u64);
                if progress.total.is_some() && bucket == self.last_bucket {
                    return;
                }
                self.last_bucket = bucket;
            }
        }

        for line in self.layout.render(event) {
            self.print(&line.text, line.end_line, &line.subject);
        }
    }

    /// 收尾：交互式下若还有一行悬着，补一个换行，别让后续输出接在进度后面。
    pub fn finish(&mut self) {
        if self.interactive && self.mid_line {
            let _ = writeln!(self.term);
            let _ = self.term.flush();
            self.mid_line = false;
        }
    }

    /// 是否是交互式终端（决定要不要弹确认）。
    pub fn is_interactive(&self) -> bool {
        self.interactive
    }

    /// `end_line=true` 是最终行（两模式都换行）；`false` 是进度行（交互式原地刷新）。
    ///
    /// `subject` 是交互式下判断"换不换行"的前缀：同一步骤 / 同一子步骤的进度
    /// 前缀不变就覆盖，换了才先换行。日志、步骤头等都走 `end_line=true`。
    fn print(&mut self, text: &str, end_line: bool, subject: &str) {
        if self.interactive {
            if self.mid_line && (end_line || subject != self.live_subject) {
                let _ = writeln!(self.term);
            }
            let _ = self.term.clear_line();
            if end_line {
                let _ = writeln!(self.term, "{text}");
                self.mid_line = false;
            } else {
                let _ = write!(self.term, "\r{text}");
                self.mid_line = true;
                self.live_subject = subject.to_string();
            }
            let _ = self.term.flush();
        } else {
            // 非交互：只有"可原地刷新"的进度行去重；日志 / 结构行逐条输出
            if !end_line && text == self.last_line {
                return;
            }
            println!("{text}");
            self.last_line = text.to_string();
        }
    }
}

/// 一条待打印的行。
struct Line {
    text: String,
    /// 最终行（两模式都换行）还是可原地刷新的进度行。
    end_line: bool,
    /// 交互模式下判断"换不换行"的前缀。
    subject: String,
}

impl Line {
    /// 一条最终行（步骤头 / 日志 / 完成 / 失败等）。
    fn final_line(text: String) -> Self {
        Self {
            text,
            end_line: true,
            subject: String::new(),
        }
    }
}

/// 层级渲染状态：栈里每层存当前步骤名，`len()` 即层级。
///
/// 与终端 I/O 解耦，方便单测。嵌套链的 `StepStarted` 入栈、`StepFinished` 出栈，
/// 内容按栈深缩进；`ctx.child(名字)` 这类同层改名子任务额外懒打一行标签。
#[derive(Default)]
struct Layout {
    stack: Vec<String>,
    /// 当前层已打印的懒标签（`ctx.child` 之类），用于避免重复打标签。
    label: Option<String>,
}

impl Layout {
    /// 把一条事件渲染成要打印的行（零行到两行）。
    fn render(&mut self, event: TaskEvent) -> Vec<Line> {
        match event {
            TaskEvent::Started { name, steps } => {
                self.stack.clear();
                self.label = None;
                let text = if steps == 0 {
                    name
                } else {
                    format!("{name}（共 {steps} 步）")
                };
                vec![Line::final_line(text)]
            }
            TaskEvent::StepStarted { step, steps, name } => {
                self.stack.push(name.clone());
                self.label = None;
                let level = self.stack.len();
                // 只有顶层步骤带 `[i/n]`；嵌套步骤名本身就是标题
                let text = if level == 1 {
                    step_header(step, steps, &name)
                } else {
                    format!("{}{name}", header_indent(level))
                };
                vec![Line::final_line(text)]
            }
            TaskEvent::Log { name, message, .. } => self.content(&name, &message, true),
            TaskEvent::Warning { name, message, .. } => {
                self.content(&name, &format!("⚠ 警告：{message}"), true)
            }
            TaskEvent::Progress { name, progress, .. } => {
                self.content(&name, &progress.to_string(), false)
            }
            TaskEvent::StepFinished { .. } => {
                self.stack.pop();
                self.label = None;
                // 只有顶层步骤结束才打「完成」，嵌套步骤靠下一个标题分隔
                if self.stack.is_empty() {
                    vec![Line::final_line(format!("{INDENT}完成"))]
                } else {
                    Vec::new()
                }
            }
            TaskEvent::Failed { step, steps, name } => {
                let level = self.stack.len().max(1);
                let text = if level == 1 {
                    format!("{} 失败", step_header(step, steps, &name))
                } else {
                    format!("{}{name} 失败", header_indent(level))
                };
                vec![Line::final_line(text)]
            }
            TaskEvent::Finished { name } => {
                self.stack.clear();
                self.label = None;
                vec![Line::final_line(format!("{name} 完成"))]
            }
        }
    }

    /// 渲染一条内容行（日志 / 警告 / 进度）。
    ///
    /// 名字等于当前步骤名时按当前层缩进；不等时（`ctx.child(名字)` 这类同层改名子
    /// 任务）先补一行标签标题，内容再深一级。
    fn content(&mut self, name: &str, body: &str, end_line: bool) -> Vec<Line> {
        let level = self.stack.len();
        let direct = level == 0 || self.stack.last().map(String::as_str) == Some(name);
        if direct {
            self.label = None;
            let indent = content_indent(level);
            return vec![Line {
                text: format!("{indent}{body}"),
                end_line,
                subject: indent,
            }];
        }

        let mut lines = Vec::new();
        if self.label.as_deref() != Some(name) {
            let indent = content_indent(level);
            lines.push(Line {
                text: format!("{indent}{name}"),
                end_line: true,
                subject: indent,
            });
            self.label = Some(name.to_string());
        }
        let indent = content_indent(level + 1);
        lines.push(Line {
            text: format!("{indent}{body}"),
            end_line,
            subject: indent,
        });
        lines
    }
}

/// 外层步骤头：`[i/n] 名字`。
fn step_header(step: usize, steps: usize, name: &str) -> String {
    format!("[{}/{}] {name}", step + 1, steps)
}

/// 步骤标题的缩进：顶层（`level == 1`）顶格，之后每层 +2 空格。
fn header_indent(level: usize) -> String {
    if level <= 1 {
        String::new()
    } else {
        format!("{INDENT}{}", NESTED.repeat(level - 2))
    }
}

/// 步骤内容的缩进：顶层 6 空格，之后每层 +2 空格。
fn content_indent(level: usize) -> String {
    format!("{INDENT}{}", NESTED.repeat(level.saturating_sub(1)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dmcl_task::Progress;

    #[test]
    fn headers_are_one_indexed() {
        assert_eq!(step_header(0, 3, "下载"), "[1/3] 下载");
        assert_eq!(step_header(2, 3, "下载"), "[3/3] 下载");
    }

    #[test]
    fn indent_grows_with_nesting_level() {
        assert_eq!(header_indent(1), "");
        assert_eq!(header_indent(2), INDENT);
        assert_eq!(header_indent(3), format!("{INDENT}  "));
        assert_eq!(content_indent(1), INDENT);
        assert_eq!(content_indent(2), format!("{INDENT}  "));
        assert_eq!(content_indent(3), format!("{INDENT}    "));
    }

    /// 嵌套链：进入下一层 push、退出 pop；内容按层级缩进，步骤名只作标题出现一次。
    #[test]
    fn nested_steps_push_pop_and_indent_their_content() {
        let mut layout = Layout::default();

        let lines = layout.render(TaskEvent::StepStarted {
            step: 0,
            steps: 1,
            name: "下载游戏文件".into(),
        });
        assert_eq!(lines[0].text, "[1/1] 下载游戏文件");

        let lines = layout.render(TaskEvent::StepStarted {
            step: 1,
            steps: 2,
            name: "下载资源".into(),
        });
        assert_eq!(lines[0].text, format!("{INDENT}下载资源"));

        let lines = layout.render(TaskEvent::Progress {
            step: 1,
            steps: 2,
            name: "下载资源".into(),
            progress: Progress::new(1, Some(2), "1/2 个文件"),
        });
        assert_eq!(
            lines[0].text,
            format!("{INDENT}  50.0% (1 B / 2 B) 1/2 个文件")
        );
        assert!(!lines[0].end_line);

        // 内层步骤结束：出栈，不打印「完成」
        assert!(layout
            .render(TaskEvent::StepFinished {
                step: 1,
                steps: 2,
                name: "下载资源".into(),
            })
            .is_empty());

        // 回到外层内容：按外层缩进
        let lines = layout.render(TaskEvent::Log {
            step: 0,
            steps: 1,
            name: "下载游戏文件".into(),
            message: "完成".into(),
        });
        assert_eq!(lines[0].text, format!("{INDENT}完成"));

        // 顶层步骤结束：出栈到空，打印「完成」
        let lines = layout.render(TaskEvent::StepFinished {
            step: 0,
            steps: 1,
            name: "下载游戏文件".into(),
        });
        assert_eq!(lines[0].text, format!("{INDENT}完成"));
    }

    /// `ctx.child(名字)` 这类同层改名子任务：首次补一行标签，内容深一级；同名字不重复。
    #[test]
    fn a_renamed_subtask_prints_one_label_then_indents_its_content() {
        let mut layout = Layout::default();
        layout.render(TaskEvent::StepStarted {
            step: 0,
            steps: 1,
            name: "运行安装器".into(),
        });

        let lines = layout.render(TaskEvent::Log {
            step: 0,
            steps: 1,
            name: "net.foo:proc".into(),
            message: "hello".into(),
        });
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, format!("{INDENT}net.foo:proc"));
        assert_eq!(lines[1].text, format!("{INDENT}  hello"));

        // 同一个名字再来：不再重复标签
        let lines = layout.render(TaskEvent::Log {
            step: 0,
            steps: 1,
            name: "net.foo:proc".into(),
            message: "world".into(),
        });
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, format!("{INDENT}  world"));

        // 回到步骤本身的内容：标签清除、恢复常规缩进
        let lines = layout.render(TaskEvent::Log {
            step: 0,
            steps: 1,
            name: "运行安装器".into(),
            message: "done".into(),
        });
        assert_eq!(lines[0].text, format!("{INDENT}done"));
    }

    /// 纯文字进度（current=0、无总量）不带没意义的 `(0 B)`。
    #[test]
    fn text_only_progress_has_no_bogus_zero_bytes() {
        let mut layout = Layout::default();
        layout.render(TaskEvent::StepStarted {
            step: 0,
            steps: 1,
            name: "解压 natives".into(),
        });
        let lines = layout.render(TaskEvent::Progress {
            step: 0,
            steps: 1,
            name: "解压 natives".into(),
            progress: Progress::new(0, None, "解压 liblwjgl.so"),
        });
        assert_eq!(lines[0].text, format!("{INDENT}解压 liblwjgl.so"));
    }

    #[test]
    fn failures_keep_the_step_header() {
        let mut layout = Layout::default();
        layout.render(TaskEvent::StepStarted {
            step: 1,
            steps: 2,
            name: "下载".into(),
        });
        let lines = layout.render(TaskEvent::Failed {
            step: 1,
            steps: 2,
            name: "下载".into(),
        });
        assert_eq!(lines[0].text, "[2/2] 下载 失败");
    }
}
