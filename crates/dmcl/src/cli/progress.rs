//! 把任务事件流渲染成终端进度。

use std::io::Write;

use dialoguer::console::{Style, Term};
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
/// [4/4] 下载游戏文件
///       获取资源索引
///         100.0% (403.4 KiB / 403.4 KiB) 1/1 个文件
///       下载库文件
///         41.2% (34.9 MiB / 84.7 MiB) 35/120 个文件
/// 安装 1.20.1 完成
/// ```
///
/// * **层级**由 [`Layout`] 用**栈**还原：嵌套链的 `StepStarted` 入栈、`StepFinished`
///   出栈，栈深即层级，嵌套步骤名只作标题出现一次、不再逐行重复。
/// * **日志弱化**：`TaskEvent::Log` 变暗（`dim`），步骤头 / 进度保持正常，突出进度。
/// * **多行内容**（含换行的消息）整块再缩进一级，成一目了然的子块。
/// * 交互式终端上，**日志打在“当前进度行”上方，进度行始终钉在最下面一行**；管道里
///   则逐行输出、跨 5% 刻度才打一条进度。
pub struct ProgressPrinter {
    term: Term,
    interactive: bool,
    /// 非交互模式：上一个进度百分比刻度。
    last_bucket: Option<u64>,
    /// 非交互模式：上一行文字（去重用）。
    last_line: String,
    /// 层级状态（栈 + 懒标签）。
    layout: Layout,
    /// 交互式：当前钉在底部的那行进度（纯文本，不含换行）。
    live: Option<String>,
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
            live: None,
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
                // 有百分比才按刻度节流；没有百分比的行靠 `last_line` 去重
                if let Some(bucket) = progress.percent.map(|percent| (percent / 5.0) as u64) {
                    if self.last_bucket == Some(bucket) {
                        return;
                    }
                    self.last_bucket = Some(bucket);
                }
            }
        }

        let lines = self.layout.render(event);
        if self.interactive {
            self.draw_interactive(&lines);
        } else {
            self.draw_plain(&lines);
        }
    }

    /// 收尾：交互式下给底部进度行补一个换行，别让后续输出接在进度后面。
    pub fn finish(&mut self) {
        if self.interactive && self.live.take().is_some() {
            let _ = writeln!(self.term);
            let _ = self.term.flush();
        }
    }

    /// 是否是交互式终端（决定要不要弹确认）。
    pub fn is_interactive(&self) -> bool {
        self.interactive
    }

    /// 非交互：行式输出；只有"可原地刷新"的进度行去重。
    fn draw_plain(&mut self, lines: &[Line]) {
        for line in lines {
            if !line.end_line && line.text == self.last_line {
                continue;
            }
            println!("{}", styled(line));
            self.last_line = line.text.clone();
        }
    }

    /// 交互式：日志等最终行打在底部进度行**上方**，进度行始终留在最下面。
    fn draw_interactive(&mut self, lines: &[Line]) {
        for line in lines {
            if line.end_line {
                // 擦掉底部进度行，把最终行打在它原来的位置
                if self.live.is_some() {
                    let _ = self.term.clear_line();
                }
                let _ = writeln!(self.term, "{}", styled(line));
                if !line.keep_live {
                    self.live = None;
                }
            } else {
                self.live = Some(line.text.clone());
            }
        }
        // 把当前进度重新画到最下面一行
        if let Some(text) = self.live.clone() {
            let _ = self.term.clear_line();
            let _ = write!(self.term, "{text}");
        }
        let _ = self.term.flush();
    }
}

/// 按标记把一行渲染成终端字符串（日志变暗；管道 / 不支持颜色时自动降级为纯文本）。
fn styled(line: &Line) -> String {
    if line.dim {
        Style::new().dim().apply_to(&line.text).to_string()
    } else {
        line.text.clone()
    }
}

/// 一条待打印的行。
struct Line {
    text: String,
    /// 最终行（两模式都换行）还是可原地刷新的进度行。
    end_line: bool,
    /// 日志弱化：用暗色渲染。
    dim: bool,
    /// 打完之后是否保留底部进度行（日志 = true；警告 / 步骤头 / 完成 / 失败 = false）。
    keep_live: bool,
}

impl Line {
    /// 一条最终行（步骤头 / 完成 / 失败等），不弱化、且丢弃底部进度行。
    fn header_line(text: String) -> Self {
        Self {
            text,
            end_line: true,
            dim: false,
            keep_live: false,
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
    /// 把一条事件渲染成要打印的行（零行到多行）。
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
                vec![Line::header_line(text)]
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
                vec![Line::header_line(text)]
            }
            TaskEvent::Log { name, message, .. } => self.content(&name, &message, true, true, true),
            TaskEvent::Warning { name, message, .. } => {
                self.content(&name, &format!("⚠ 警告：{message}"), true, false, false)
            }
            TaskEvent::Progress { name, progress, .. } => {
                self.content(&name, &progress.to_string(), false, false, false)
            }
            TaskEvent::StepFinished { .. } => {
                // 只出栈，不打「完成」：每个步骤都打一行太吵，链尾的 `Finished` 已经
                // 给出了总的完成提示
                self.stack.pop();
                self.label = None;
                Vec::new()
            }
            TaskEvent::Failed { step, steps, name } => {
                let level = self.stack.len().max(1);
                let text = if level == 1 {
                    format!("{} 失败", step_header(step, steps, &name))
                } else {
                    format!("{}{name} 失败", header_indent(level))
                };
                vec![Line::header_line(text)]
            }
            TaskEvent::Finished { name } => {
                self.stack.clear();
                self.label = None;
                vec![Line::header_line(format!("{name} 完成"))]
            }
        }
    }

    /// 渲染一条内容（日志 / 警告 / 进度）。
    ///
    /// 名字等于当前步骤名时按当前层缩进；不等时（`ctx.child(名字)` 这类同层改名子
    /// 任务）先补一行标签标题，内容再深一级。**含换行的消息整块再缩进一级**。
    fn content(
        &mut self,
        name: &str,
        body: &str,
        end_line: bool,
        dim: bool,
        keep_live: bool,
    ) -> Vec<Line> {
        let level = self.stack.len();
        let direct = level == 0 || self.stack.last().map(String::as_str) == Some(name);
        let mut lines = Vec::new();
        let indent = if direct {
            self.label = None;
            content_indent(level)
        } else {
            if self.label.as_deref() != Some(name) {
                let label_indent = content_indent(level);
                lines.push(Line {
                    text: format!("{label_indent}{name}"),
                    end_line: true,
                    dim,
                    keep_live,
                });
                self.label = Some(name.to_string());
            }
            content_indent(level + 1)
        };

        // 多行内容整块再缩进一级；单行按当前缩进
        let indent = if body.contains('\n') {
            content_indent(level + 1)
        } else {
            indent
        };
        if body.is_empty() {
            lines.push(Line {
                text: indent,
                end_line,
                dim,
                keep_live,
            });
        } else {
            for text in body.lines() {
                lines.push(Line {
                    text: format!("{indent}{text}"),
                    end_line,
                    dim,
                    keep_live,
                });
            }
        }
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
            progress: Progress::new(1, Some(2), "1/2 个文件", Some(50.0)),
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

        // 顶层步骤结束：出栈，不打「完成」（完成只在链尾由 `Finished` 打一次）
        let lines = layout.render(TaskEvent::StepFinished {
            step: 0,
            steps: 1,
            name: "下载游戏文件".into(),
        });
        assert!(lines.is_empty());

        let lines = layout.render(TaskEvent::Finished {
            name: "启动".into(),
        });
        assert_eq!(lines[0].text, "启动 完成");
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

    /// 含换行的日志整块再缩进一级，且每一行都变暗。
    #[test]
    fn a_multi_line_log_becomes_an_indented_block() {
        let mut layout = Layout::default();
        layout.render(TaskEvent::StepStarted {
            step: 0,
            steps: 1,
            name: "检查".into(),
        });
        let lines = layout.render(TaskEvent::Log {
            step: 0,
            steps: 1,
            name: "检查".into(),
            message: "第一行\n第二行".into(),
        });
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, format!("{INDENT}  第一行"));
        assert_eq!(lines[1].text, format!("{INDENT}  第二行"));
        assert!(lines.iter().all(|line| line.dim && line.end_line));
    }

    /// 日志弱化（dim），进度与步骤头保持正常。
    #[test]
    fn logs_are_dimmed_but_headers_and_progress_are_not() {
        let mut layout = Layout::default();
        let header = layout.render(TaskEvent::StepStarted {
            step: 0,
            steps: 1,
            name: "下载".into(),
        });
        assert!(!header[0].dim);

        let log = layout.render(TaskEvent::Log {
            step: 0,
            steps: 1,
            name: "下载".into(),
            message: "hello".into(),
        });
        assert!(log[0].dim);

        let progress = layout.render(TaskEvent::Progress {
            step: 0,
            steps: 1,
            name: "下载".into(),
            progress: Progress::new(1, Some(2), "1/2 个文件", Some(50.0)),
        });
        assert!(!progress[0].dim);
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
            progress: Progress::new(0, None, "解压 liblwjgl.so", None),
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
