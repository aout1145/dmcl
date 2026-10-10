//! 把任务事件流渲染成终端进度。

use std::io::Write;

use dialoguer::console::Term;
use dmcl_task::TaskEvent;

/// 进度打印机。
///
/// * 终端上：同一步骤的进度**原地刷新一行**，步骤切换时才换行；
/// * 管道 / 日志里：只在跨过 5% 的刻度时打印一行，避免刷屏。
pub struct ProgressPrinter {
    term: Term,
    interactive: bool,
    last_bucket: Option<u64>,
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
        }
    }

    /// 直接交给 `TaskChain::run_with`。
    pub fn handle(&mut self, event: TaskEvent) {
        match event {
            TaskEvent::Started { name, steps } => {
                self.last_bucket = None;
                self.line(&format!("{name}（{steps} 个步骤）"), true);
            }
            TaskEvent::StepStarted { step, steps, name } => {
                self.last_bucket = None;
                self.line(&format!("[{}/{}] {name}", step + 1, steps), false);
            }
            TaskEvent::Progress {
                step,
                steps,
                name,
                progress,
            } => {
                let bucket = progress.percent().map(|percent| (percent / 5.0) as u64);
                if self.interactive || bucket != self.last_bucket {
                    self.last_bucket = bucket;
                    self.line(
                        &format!("[{}/{}] {name}: {progress}", step + 1, steps),
                        false,
                    );
                }
            }
            TaskEvent::StepFinished { step, steps, name } => {
                // 交互式下这一行马上会被下一步覆盖，不必单独打印
                if !self.interactive {
                    self.line(&format!("[{}/{}] {name}: 完成", step + 1, steps), true);
                }
            }
            TaskEvent::Failed {
                step,
                steps,
                name,
                error,
            } => {
                self.line(
                    &format!("[{}/{}] {name} 失败: {error}", step + 1, steps),
                    true,
                );
            }
            TaskEvent::Finished { name } => self.line(&format!("{name} 完成"), true),
        }
    }

    /// 换行，确保后面的普通输出不会接在进度行后面。
    pub fn finish(&mut self) {
        if self.interactive {
            let _ = self.term.clear_line();
            let _ = writeln!(self.term);
            let _ = self.term.flush();
        }
    }

    fn line(&mut self, text: &str, end_line: bool) {
        if self.interactive {
            let _ = self.term.clear_line();
            let _ = write!(self.term, "\r{text}");
            if end_line {
                let _ = writeln!(self.term);
                self.last_bucket = None;
            }
            let _ = self.term.flush();
        } else {
            println!("{text}");
        }
    }
}
