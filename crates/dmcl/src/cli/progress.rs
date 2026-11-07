//! 把任务事件流渲染成终端进度。

use std::io::Write;

use dialoguer::console::{Style, Term};
use dmcl_task::{human_bytes, StepDef, TaskEvent, TaskRef};

/// 一个 Step 的计数。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Counts {
    waiting: u64,
    running: u64,
    /// 成功 + 失败 + 跳过。
    completed: u64,
    /// 失败数（包含在 `completed` 里）。
    failed: u64,
}

/// 进度打印机：Step 计数 + 一条全局进度行。
pub struct ProgressPrinter {
    term: Term,
    interactive: bool,
    name: String,
    steps: Vec<StepDef>,
    counts: Vec<Counts>,
    /// 非交互模式：上一个进度百分比刻度。
    last_bucket: Option<u64>,
    /// 非交互模式：上一行文字（去重用）。
    last_line: String,
    /// 交互式：当前钉在底部的那行进度。
    live: Option<String>,
    downloaded: u64,
    total: u64,
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
            name: String::new(),
            steps: Vec::new(),
            counts: Vec::new(),
            last_bucket: None,
            last_line: String::new(),
            live: None,
            downloaded: 0,
            total: 0,
        }
    }

    /// 直接交给 `TaskGraph::run`。
    pub fn handle(&mut self, event: TaskEvent) {
        let mut lines: Vec<(String, bool)> = Vec::new();
        let mut update_live = true;

        match event {
            TaskEvent::GraphStarted { name, steps } => {
                self.name = name.clone();
                self.steps = steps;
                self.counts = vec![Counts::default(); self.steps.len()];
                self.last_bucket = None;
                self.last_line.clear();
                self.downloaded = 0;
                self.total = 0;
                lines.push((name, false));
            }
            TaskEvent::TaskAdded { task } => {
                self.counts_mut(&task).waiting += 1;
            }
            TaskEvent::TaskStarted { task } => {
                let counts = self.counts_mut(&task);
                counts.waiting = counts.waiting.saturating_sub(1);
                counts.running += 1;
            }
            TaskEvent::TaskFinished { task } => {
                let counts = self.counts_mut(&task);
                counts.running = counts.running.saturating_sub(1);
                counts.completed += 1;
            }
            TaskEvent::TaskFailed { task } => {
                let counts = self.counts_mut(&task);
                counts.running = counts.running.saturating_sub(1);
                counts.completed += 1;
                counts.failed += 1;
                lines.push((format!("{}: 失败", task.name()), false));
            }
            TaskEvent::TaskSkipped { task } => {
                let counts = self.counts_mut(&task);
                counts.waiting = counts.waiting.saturating_sub(1);
                counts.completed += 1;
            }
            TaskEvent::Progress { downloaded, total } => {
                self.downloaded = downloaded;
                self.total = total;
                if !self.interactive {
                    let bucket = (downloaded.saturating_mul(100) / total.max(1)).min(100) / 5;
                    if total > 0 && self.last_bucket == Some(bucket) {
                        update_live = false;
                    } else {
                        self.last_bucket = Some(bucket);
                        lines.push((progress_text(downloaded, total), false));
                    }
                }
            }
            TaskEvent::Log { task, message } => {
                let text = match &task {
                    Some(task) => format!("{}: {message}", task.name()),
                    None => message,
                };
                lines.push((text, true));
            }
            TaskEvent::Warning { task, message, .. } => {
                let text = match &task {
                    Some(task) => format!("{}: ⚠ 警告：{message}", task.name()),
                    None => format!("⚠ 警告：{message}"),
                };
                lines.push((text, false));
            }
            TaskEvent::GraphFinished => {
                lines.extend(self.step_lines().into_iter().map(|line| (line, false)));
                lines.push((format!("{} 完成", self.name), false));
            }
            TaskEvent::GraphFailed => {
                lines.extend(self.step_lines().into_iter().map(|line| (line, false)));
                lines.push((format!("{} 失败", self.name), false));
            }
        }

        if self.interactive {
            if update_live {
                self.live = self.live_text();
            }
            self.draw_interactive(&lines);
        } else {
            self.draw_plain(&lines);
        }
    }

    /// 收尾：交互式下给底部进度行补一个换行。
    pub fn finish(&mut self) {
        if self.interactive && self.live.take().is_some() {
            let _ = writeln!(self.term);
            let _ = self.term.flush();
        }
    }

    pub fn is_interactive(&self) -> bool {
        self.interactive
    }

    fn step_lines(&self) -> Vec<String> {
        self.steps
            .iter()
            .enumerate()
            .map(|(index, step)| {
                let counts = self.counts.get(index).copied().unwrap_or_default();
                let mut text = format!(
                    "{}: 等待 {} · 执行 {} · 完成 {}",
                    step.name, counts.waiting, counts.running, counts.completed
                );
                if counts.failed > 0 {
                    text.push_str(&format!(" · 失败 {}", counts.failed));
                }
                text
            })
            .collect()
    }

    fn counts_mut(&mut self, task: &TaskRef) -> &mut Counts {
        let index = self
            .steps
            .iter()
            .position(|step| step.id == task.step())
            .unwrap_or(0);
        if self.counts.len() <= index {
            self.counts.resize(index + 1, Counts::default());
        }
        &mut self.counts[index]
    }

    fn live_text(&self) -> Option<String> {
        self.last_progress_text()
    }

    fn last_progress_text(&self) -> Option<String> {
        (self.downloaded > 0 || self.total > 0).then(|| progress_text(self.downloaded, self.total))
    }

    fn draw_plain(&mut self, lines: &[(String, bool)]) {
        for (text, dim) in lines {
            if text == &self.last_line {
                continue;
            }
            if *dim {
                println!("{}", Style::new().dim().apply_to(text));
            } else {
                println!("{text}");
            }
            self.last_line = text.clone();
        }
    }

    fn draw_interactive(&mut self, lines: &[(String, bool)]) {
        for (text, dim) in lines {
            if self.live.is_some() {
                let _ = self.term.clear_line();
            }
            if *dim {
                let _ = writeln!(self.term, "{}", Style::new().dim().apply_to(text));
            } else {
                let _ = writeln!(self.term, "{text}");
            }
        }
        if let Some(live) = &self.live {
            let _ = self.term.clear_line();
            let _ = write!(self.term, "{live}");
        }
        let _ = self.term.flush();
    }
}

fn progress_text(downloaded: u64, total: u64) -> String {
    if total == 0 || downloaded >= total {
        format!("进度：{}", human_bytes(downloaded))
    } else {
        let percent = downloaded as f64 / total as f64 * 100.0;
        format!(
            "进度：{} / {}（{percent:.0}%）",
            human_bytes(downloaded),
            human_bytes(total)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_text_handles_zero_and_complete() {
        assert_eq!(progress_text(0, 0), "进度：0 B");
        assert_eq!(progress_text(1024, 1024), "进度：1.0 KiB");
        assert_eq!(progress_text(512, 1024), "进度：512 B / 1.0 KiB（50%）");
    }

    #[test]
    fn counts_can_be_updated_independently() {
        let mut counts = Counts::default();
        counts.waiting += 2;
        counts.waiting -= 1;
        counts.running += 1;
        counts.running -= 1;
        counts.completed += 1;
        counts.failed += 1;
        assert_eq!(
            counts,
            Counts {
                waiting: 1,
                running: 0,
                completed: 1,
                failed: 1,
            }
        );
    }
}
