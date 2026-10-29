//! 任务卡片的状态与进度归约（纯逻辑，可单测）。
//!
//! 嵌套任务链会把子链的步骤事件**原样转发**到父链，所以总进度不能用 `StepFinished`
//! 直接计数——要用**栈**还原层级，只统计**顶层**步骤的完成：
//! 栈由空变非空 = 一个顶层步骤开始；由非空变空 = 一个顶层步骤完成。

use dmcl_task::{human_bytes, Progress, TaskEvent};

/// 任务的运行状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TaskStatus {
    /// 正在运行。
    Running,
    /// 已经请求取消，正在等任务收尾。
    Canceling,
    /// 正常完成。
    Done,
    /// 被用户取消。
    Cancelled,
    /// 失败。
    Failed,
}

/// 一个任务的展示状态。
#[derive(Debug, Clone)]
pub(crate) struct TaskState {
    pub name: String,
    pub total_steps: usize,
    pub done_steps: usize,
    /// 当前活跃的步骤名（从顶层到最内层）。
    stack: Vec<String>,
    /// 最近一次分项进度。
    pub current: Option<Progress>,
    pub logs: Vec<String>,
    pub status: TaskStatus,
    /// 失败原因（来自任务链的返回值；里面已经带了出错的步骤名）。
    pub error: Option<String>,
}

impl TaskState {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            total_steps: 0,
            done_steps: 0,
            stack: Vec::new(),
            current: None,
            logs: Vec::new(),
            status: TaskStatus::Running,
            error: None,
        }
    }

    /// 应用一条事件。
    pub fn apply(&mut self, event: &TaskEvent) {
        match event {
            TaskEvent::Started { steps, .. } => {
                // 名字保留前端传入的那个（比链自己的名字更具体，例如「启动 1.20.1」）。
                self.total_steps = *steps;
                self.done_steps = 0;
                self.stack.clear();
                self.current = None;
            }
            TaskEvent::StepStarted { name, .. } => {
                self.stack.push(name.clone());
                self.current = None;
            }
            TaskEvent::StepFinished { .. } => {
                self.stack.pop();
                if self.stack.is_empty() {
                    self.done_steps += 1;
                }
            }
            TaskEvent::Log { message, .. } => self.logs.push(message.clone()),
            TaskEvent::Warning { message, .. } => self.logs.push(format!("⚠ 警告：{message}")),
            TaskEvent::Progress { progress, .. } => self.current = Some(progress.clone()),
            TaskEvent::Failed { .. } => {
                // 已经请求取消时保持「取消中」：接下来只会收到取消造成的失败，
                // 不要让界面先闪一下「失败」。
                if self.status != TaskStatus::Canceling {
                    self.status = TaskStatus::Failed;
                }
            }
            TaskEvent::Finished { .. } => {}
        }
    }

    /// 请求取消：正在运行时进入「取消中」。
    pub fn request_cancel(&mut self) {
        if self.status == TaskStatus::Running {
            self.status = TaskStatus::Canceling;
        }
    }

    /// 任务结束（`run` / `run_with` 的返回值）。
    pub fn finish(&mut self, ok: bool, cancelled: bool, error: Option<String>) {
        self.status = if cancelled {
            TaskStatus::Cancelled
        } else if ok {
            TaskStatus::Done
        } else {
            TaskStatus::Failed
        };
        self.error = if ok || cancelled { None } else { error };
        self.stack.clear();
        self.current = None;
    }

    /// 是否已经结束（成功、失败或被取消）。
    pub fn is_finished(&self) -> bool {
        matches!(
            self.status,
            TaskStatus::Done | TaskStatus::Cancelled | TaskStatus::Failed
        )
    }

    /// 总进度：已完成步骤数 / 总步骤数。
    pub fn overall(&self) -> (usize, usize) {
        (self.done_steps, self.total_steps)
    }

    /// 当前正在执行的顶层步骤序号（1 起，按总步骤数封顶）。
    ///
    /// 事件里的 `step` 是 0 起；界面显示「第几步」时用这个值。
    pub fn current_step_number(&self) -> usize {
        (self.done_steps + 1).min(self.total_steps)
    }

    /// 当前最内层步骤名。
    pub fn current_step(&self) -> Option<&str> {
        self.stack.last().map(String::as_str)
    }

    /// 分项进度里可用的百分比（没有就是 `None`）。
    pub fn current_percent(&self) -> Option<f64> {
        self.current.as_ref().and_then(|progress| progress.percent)
    }

    /// 进度条上方的一行说明：百分比 / 字节数 / 任务自己的计数文字。
    ///
    /// 例如下载步骤是 `41.2% · 34.9 MiB / 84.7 MiB · 35/120 个文件`。
    pub fn progress_text(&self) -> String {
        let Some(progress) = &self.current else {
            return String::new();
        };
        let mut parts = Vec::new();
        if let Some(percent) = progress.percent {
            parts.push(format!("{percent:.1}%"));
        }
        match progress.total {
            Some(total) if total > 0 || progress.current > 0 => parts.push(format!(
                "{} / {}",
                human_bytes(progress.current),
                human_bytes(total)
            )),
            None if progress.current > 0 => parts.push(human_bytes(progress.current)),
            _ => {}
        }
        if !progress.message.is_empty() {
            parts.push(progress.message.clone());
        }
        parts.join(" · ")
    }

    /// 是否该用 pulse 当「不确定进度条」：运行中、有当前步骤、但没有百分比。
    pub fn wants_pulse(&self) -> bool {
        self.status == TaskStatus::Running
            && self.current_step().is_some()
            && self.current_percent().is_none()
    }

    /// 日志全文。
    pub fn logs_text(&self) -> String {
        self.logs.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn started(steps: usize) -> TaskEvent {
        TaskEvent::Started {
            name: "任务".into(),
            steps,
        }
    }

    fn step_started(name: &str) -> TaskEvent {
        TaskEvent::StepStarted {
            step: 0,
            steps: 1,
            name: name.into(),
        }
    }

    fn step_finished(name: &str) -> TaskEvent {
        TaskEvent::StepFinished {
            step: 0,
            steps: 1,
            name: name.into(),
        }
    }

    /// 只统计顶层步骤：嵌套子链转发来的步骤不算数。
    #[test]
    fn counts_only_top_level_steps() {
        let mut state = TaskState::new("t");
        state.apply(&started(2));
        assert_eq!(state.current_step_number(), 1);

        state.apply(&step_started("A"));
        // A 内部又是一个链：子步骤开始 / 结束
        state.apply(&step_started("child"));
        state.apply(&step_finished("child"));
        assert_eq!(state.overall(), (0, 2), "子步骤结束不该计入总进度");
        assert_eq!(state.current_step_number(), 1);

        state.apply(&step_finished("A"));
        assert_eq!(state.overall(), (1, 2), "顶层步骤 A 结束才 +1");

        // 下一步开始时，界面应当显示「第 2 步」，而不是已完成的 1。
        state.apply(&step_started("B"));
        assert_eq!(state.current_step_number(), 2);
        assert_eq!(state.current_step(), Some("B"));
        state.apply(&step_finished("B"));
        assert_eq!(state.overall(), (2, 2));
        assert_eq!(state.current_step_number(), 2);
    }

    /// 分项进度取最近一条；当前步骤变了要清掉旧的进度。
    #[test]
    fn progress_tracks_the_innermost_step() {
        let mut state = TaskState::new("t");
        state.apply(&started(1));
        state.apply(&step_started("下载"));
        assert_eq!(state.current_step(), Some("下载"));
        assert_eq!(state.current_percent(), None);

        state.apply(&TaskEvent::Progress {
            step: 0,
            steps: 1,
            name: "下载".into(),
            progress: Progress::new(1, Some(2), "a.jar", Some(50.0)),
        });
        assert_eq!(state.current_percent(), Some(50.0));

        // 换步骤：进度清零
        state.apply(&step_finished("下载"));
        state.apply(&step_started("解压"));
        assert_eq!(state.current_percent(), None);
    }

    /// 日志按顺序累积，警告带前缀。
    #[test]
    fn logs_accumulate_in_order() {
        let mut state = TaskState::new("t");
        state.apply(&TaskEvent::Log {
            step: 0,
            steps: 1,
            name: "x".into(),
            message: "第一行".into(),
        });
        state.apply(&TaskEvent::Log {
            step: 0,
            steps: 1,
            name: "x".into(),
            message: "第二行".into(),
        });
        assert_eq!(state.logs_text(), "第一行\n第二行");
    }

    /// 结束标记状态，并记录失败原因。
    #[test]
    fn finish_sets_status_and_clears_running_step() {
        let mut state = TaskState::new("t");
        state.apply(&started(1));
        state.apply(&step_started("A"));

        state.finish(false, false, Some("步骤「A」失败：boom".into()));
        assert_eq!(state.status, TaskStatus::Failed);
        assert_eq!(state.error.as_deref(), Some("步骤「A」失败：boom"));
        assert_eq!(state.current_step(), None);

        state.finish(true, false, None);
        assert_eq!(state.status, TaskStatus::Done);
        assert_eq!(state.error, None);
    }

    /// 请求取消后进入「取消中」；取消造成的失败不覆盖这个状态，最后落到「已取消」。
    #[test]
    fn cancelling_is_kept_until_the_task_finishes() {
        let mut state = TaskState::new("t");
        state.apply(&started(1));
        state.apply(&step_started("下载"));

        state.request_cancel();
        assert_eq!(state.status, TaskStatus::Canceling);
        assert!(!state.is_finished());

        // 任务在被取消后可能会先发一条 Failed 事件。
        state.apply(&TaskEvent::Failed {
            step: 0,
            steps: 1,
            name: "下载".into(),
        });
        assert_eq!(state.status, TaskStatus::Canceling);

        state.finish(false, true, Some("任务已取消".into()));
        assert_eq!(state.status, TaskStatus::Cancelled);
        assert_eq!(state.error, None);
        assert!(state.is_finished());
    }

    /// 进度说明包含百分比、字节和任务自己的计数（下载的文件数）。
    #[test]
    fn progress_text_combines_percent_bytes_and_counts() {
        let mut state = TaskState::new("t");
        state.apply(&started(1));
        state.apply(&step_started("下载"));
        state.apply(&TaskEvent::Progress {
            step: 0,
            steps: 1,
            name: "下载".into(),
            progress: Progress::new(2, Some(4), "2/3 个文件", Some(50.0)),
        });
        assert_eq!(state.progress_text(), "50.0% · 2 B / 4 B · 2/3 个文件");
    }

    /// 链自己的名字（如「启动」）不能覆盖前端给的展示名（「启动 1.20.1」）。
    #[test]
    fn the_display_name_survives_the_started_event() {
        let mut state = TaskState::new("启动 1.20.1");
        state.apply(&TaskEvent::Started {
            name: "启动".into(),
            steps: 2,
        });
        assert_eq!(state.name, "启动 1.20.1");
        assert_eq!(state.total_steps, 2);
    }
}
