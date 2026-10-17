//! 任务卡片的状态与进度归约（纯逻辑，可单测）。
//!
//! 嵌套任务链会把子链的步骤事件**原样转发**到父链，所以总进度不能用 `StepFinished`
//! 直接计数——要用**栈**还原层级，只统计**顶层**步骤的完成：
//! 栈由空变非空 = 一个顶层步骤开始；由非空变空 = 一个顶层步骤完成。

use dmcl_task::{Progress, TaskEvent};

/// 任务的运行状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TaskStatus {
    Running,
    Done,
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
            TaskEvent::Failed { .. } => self.status = TaskStatus::Failed,
            TaskEvent::Finished { .. } => {}
        }
    }

    /// 任务结束（`run` / `run_with` 的返回值）。
    pub fn finish(&mut self, ok: bool, error: Option<String>) {
        self.status = if ok {
            TaskStatus::Done
        } else {
            TaskStatus::Failed
        };
        self.error = if ok { None } else { error };
        self.stack.clear();
        self.current = None;
    }

    /// 总进度：已完成步骤数 / 总步骤数。
    pub fn overall(&self) -> (usize, usize) {
        (self.done_steps, self.total_steps)
    }

    /// 当前最内层步骤名。
    pub fn current_step(&self) -> Option<&str> {
        self.stack.last().map(String::as_str)
    }

    /// 分项进度里可用的百分比（没有就是 `None`）。
    pub fn current_percent(&self) -> Option<f64> {
        self.current.as_ref().and_then(|progress| progress.percent)
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

        state.apply(&step_started("A"));
        // A 内部又是一个链：子步骤开始 / 结束
        state.apply(&step_started("child"));
        state.apply(&step_finished("child"));
        assert_eq!(state.overall(), (0, 2), "子步骤结束不该计入总进度");

        state.apply(&step_finished("A"));
        assert_eq!(state.overall(), (1, 2), "顶层步骤 A 结束才 +1");
        assert_eq!(state.current_step(), None);

        state.apply(&step_started("B"));
        assert_eq!(state.current_step(), Some("B"));
        state.apply(&step_finished("B"));
        assert_eq!(state.overall(), (2, 2));
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

        state.finish(false, Some("步骤「A」失败：boom".into()));
        assert_eq!(state.status, TaskStatus::Failed);
        assert_eq!(state.error.as_deref(), Some("步骤「A」失败：boom"));
        assert_eq!(state.current_step(), None);

        state.finish(true, None);
        assert_eq!(state.status, TaskStatus::Done);
        assert_eq!(state.error, None);
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
