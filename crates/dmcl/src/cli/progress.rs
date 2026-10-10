//! 把任务事件流渲染成终端进度。

use std::io::Write;

use dialoguer::console::Term;
use dmcl_task::TaskEvent;

/// 进度打印机。
///
/// 渲染原则：
///
/// * **每个步骤占一行**：交互式终端上同一行的进度原地刷新，步骤结束时把这一行
///   收尾成「完成」；行首固定是 `[i/n] 步骤名`，进度只接在后面，不再把文件名
///   重复打印两遍；
/// * **管道 / 日志里**：每跨过 5% 的刻度才打一行，纯文字状态消息才逐条打印
///   （并去重），避免几百个文件刷屏。
pub struct ProgressPrinter {
    term: Term,
    interactive: bool,
    /// 非交互模式：上一个进度百分比刻度。
    last_bucket: Option<u64>,
    /// 非交互模式：上一行文字（去重用）。
    last_line: String,
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
            mid_line: false,
        }
    }

    /// 直接交给 `TaskChain::run_with`。
    pub fn handle(&mut self, event: TaskEvent) {
        // 非交互模式下的节流：字节进度只在跨 5% 刻度时打，纯文字状态照常
        if !self.interactive {
            if let TaskEvent::Progress { ref progress, .. } = event {
                let bucket = progress.percent().map(|percent| (percent / 5.0) as u64);
                if progress.total.is_some() && bucket == self.last_bucket {
                    return;
                }
                self.last_bucket = bucket;
            } else {
                self.last_bucket = None;
            }
        }

        let text = render(&event);
        if !self.interactive && text == self.last_line {
            return;
        }

        // 进度行在交互式下悬着（原地刷新），其余都是最终行
        let end_line = !self.interactive || !matches!(event, TaskEvent::Progress { .. });
        self.print(&text, end_line);
    }

    /// 收尾：交互式下若还有一行悬着，补一个换行，别让后续输出接在进度后面。
    pub fn finish(&mut self) {
        if self.interactive && self.mid_line {
            let _ = writeln!(self.term);
            let _ = self.term.flush();
            self.mid_line = false;
        }
    }

    fn print(&mut self, text: &str, end_line: bool) {
        if self.interactive {
            let _ = self.term.clear_line();
            if end_line {
                let _ = writeln!(self.term, "{text}");
                self.mid_line = false;
            } else {
                let _ = write!(self.term, "\r{text}");
                self.mid_line = true;
            }
            let _ = self.term.flush();
        } else {
            println!("{text}");
            self.last_line = text.to_string();
        }
    }
}

/// 把一个事件渲染成要显示的那一行（纯函数，便于单测）。
fn render(event: &TaskEvent) -> String {
    match event {
        TaskEvent::Started { name, steps } => {
            if *steps == 0 {
                name.clone()
            } else {
                format!("{name}（共 {steps} 步）")
            }
        }
        TaskEvent::StepStarted { step, steps, name } => header(*step, *steps, name),
        TaskEvent::Progress {
            step,
            steps,
            name,
            progress,
        } => format!("{}  {progress}", header(*step, *steps, name)),
        TaskEvent::StepFinished { step, steps, name } => {
            format!("{} 完成", header(*step, *steps, name))
        }
        TaskEvent::Failed {
            step,
            steps,
            name,
            error,
        } => format!("{} 失败：{error}", header(*step, *steps, name)),
        TaskEvent::Finished { name } => format!("{name} 完成"),
    }
}

/// `[i/n] 名字`。
fn header(step: usize, steps: usize, name: &str) -> String {
    format!("[{}/{}] {name}", step + 1, steps)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dmcl_task::Progress;

    #[test]
    fn headers_are_one_indexed() {
        assert_eq!(header(0, 3, "下载"), "[1/3] 下载");
        assert_eq!(header(2, 3, "下载"), "[3/3] 下载");
    }

    #[test]
    fn byte_progress_is_rendered_on_a_single_line_without_duplicating_names() {
        let event = TaskEvent::Progress {
            step: 1,
            steps: 4,
            name: "下载库文件".to_string(),
            progress: Progress::new(10, Some(20), "35/120 个文件".to_string()),
        };
        assert_eq!(
            render(&event),
            "[2/4] 下载库文件   50.0% (10 B / 20 B) 35/120 个文件"
        );
    }

    #[test]
    fn status_only_progress_does_not_print_a_bogus_zero_bytes() {
        let event = TaskEvent::Progress {
            step: 0,
            steps: 1,
            name: "解压 natives".to_string(),
            progress: Progress::status("解压 liblwjgl.so".to_string()),
        };
        assert_eq!(render(&event), "[1/1] 解压 natives  解压 liblwjgl.so");
    }

    #[test]
    fn step_boundaries_and_failures_are_rendered() {
        assert_eq!(
            render(&TaskEvent::StepStarted {
                step: 0,
                steps: 2,
                name: "下载".to_string()
            }),
            "[1/2] 下载"
        );
        assert_eq!(
            render(&TaskEvent::StepFinished {
                step: 1,
                steps: 2,
                name: "下载".to_string()
            }),
            "[2/2] 下载 完成"
        );
        assert_eq!(
            render(&TaskEvent::Failed {
                step: 1,
                steps: 2,
                name: "下载".to_string(),
                error: "404".to_string()
            }),
            "[2/2] 下载 失败：404"
        );
        assert_eq!(
            render(&TaskEvent::Finished {
                name: "安装".to_string()
            }),
            "安装 完成"
        );
    }
}
