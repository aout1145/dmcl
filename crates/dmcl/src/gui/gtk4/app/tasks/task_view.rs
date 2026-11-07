//! 任务卡片的状态与进度归约（纯逻辑，可单测）。
//!
//! 图的 Step 是静态的：`GraphStarted` 一次性带出全部 Step，之后只按任务事件累计
//! 每个 Step 的等待 / 执行 / 完成计数；全局进度只有 `downloaded / total`。

use dmcl_task::{human_bytes, StepDef, TaskEvent, TaskRef};

/// 任务的运行状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TaskStatus {
    Running,
    Canceling,
    Done,
    Cancelled,
    Failed,
}

/// 一个 Step 的计数。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct StepCounts {
    waiting: u64,
    running: u64,
    completed: u64,
    failed: u64,
    /// 图被取消 / 失败时还没开始的任务。
    skipped: u64,
}

impl StepCounts {
    fn active(&self) -> bool {
        self.waiting + self.running > 0
    }

    fn total(&self) -> u64 {
        self.waiting + self.running + self.completed + self.skipped
    }
}

/// 一个 Step 在界面上的一行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StepView {
    pub number: usize,
    pub name: String,
    pub status: StepDisplayStatus,
    pub waiting: u64,
    pub running: u64,
    pub completed: u64,
    pub failed: u64,
    pub skipped: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StepDisplayStatus {
    Waiting,
    Running,
    Done,
    /// 有任务没执行（取消 / 失败后仍停在等待），或者整个 Step 没有任何任务。
    Incomplete,
    Failed,
}

impl StepView {
    fn new(number: usize, name: String, counts: StepCounts, finished: bool) -> Self {
        let status = if counts.running > 0 {
            StepDisplayStatus::Running
        } else if counts.failed > 0 {
            StepDisplayStatus::Failed
        } else if counts.waiting > 0 {
            if finished {
                StepDisplayStatus::Incomplete
            } else {
                StepDisplayStatus::Waiting
            }
        } else if counts.skipped > 0 {
            StepDisplayStatus::Incomplete
        } else if counts.total() == 0 {
            if finished {
                StepDisplayStatus::Incomplete
            } else {
                StepDisplayStatus::Waiting
            }
        } else {
            StepDisplayStatus::Done
        };
        Self {
            number,
            name,
            status,
            waiting: counts.waiting,
            running: counts.running,
            completed: counts.completed,
            failed: counts.failed,
            skipped: counts.skipped,
        }
    }

    fn total(&self) -> u64 {
        self.waiting + self.running + self.completed + self.skipped
    }

    pub fn title(&self) -> String {
        format!("步骤 {}  {}", self.number, self.name)
    }

    pub fn icon_name(&self) -> &'static str {
        match self.status {
            StepDisplayStatus::Done => "object-select-symbolic",
            StepDisplayStatus::Running => "content-loading-symbolic",
            StepDisplayStatus::Waiting | StepDisplayStatus::Incomplete => "radio-symbolic",
            StepDisplayStatus::Failed => "dialog-error-symbolic",
        }
    }

    pub fn icon_classes(&self) -> Vec<&'static str> {
        match self.status {
            StepDisplayStatus::Done => vec!["success"],
            StepDisplayStatus::Running => vec!["accent"],
            StepDisplayStatus::Waiting | StepDisplayStatus::Incomplete => vec!["dim-label"],
            StepDisplayStatus::Failed => vec!["error"],
        }
    }

    pub fn name_classes(&self) -> Vec<&'static str> {
        match self.status {
            StepDisplayStatus::Waiting | StepDisplayStatus::Incomplete => {
                vec!["dmcl-step-title", "dim-label"]
            }
            StepDisplayStatus::Failed => vec!["dmcl-step-title", "error"],
            StepDisplayStatus::Running | StepDisplayStatus::Done => vec!["dmcl-step-title"],
        }
    }

    pub fn counts_text(&self) -> String {
        let total = self.total();
        let mut text = match self.status {
            StepDisplayStatus::Running => format!("{}/{total} 运行中", self.completed),
            StepDisplayStatus::Waiting if total == 0 => "等待中".to_string(),
            StepDisplayStatus::Waiting => format!("{}/{total} 等待中", self.completed),
            StepDisplayStatus::Done => format!("{}/{total} 完成", self.completed),
            // 有完成也有未执行：仍显示完成比例，但整体是灰色的。
            StepDisplayStatus::Incomplete if self.completed > 0 => {
                format!("{}/{total} 完成", self.completed)
            }
            StepDisplayStatus::Incomplete => "未执行".to_string(),
            StepDisplayStatus::Failed => format!("{}/{total} 失败", self.completed),
        };
        if self.failed > 0 && self.status != StepDisplayStatus::Failed {
            text.push_str(&format!(" · {} 失败", self.failed));
        }
        text
    }
}

/// 一个任务的展示状态。
#[derive(Debug, Clone)]
pub(crate) struct TaskState {
    pub name: String,
    steps: Vec<StepDef>,
    counts: Vec<StepCounts>,
    pub downloaded: u64,
    pub total: u64,
    pub logs: Vec<String>,
    pub status: TaskStatus,
    pub error: Option<String>,
}

impl TaskState {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            steps: Vec::new(),
            counts: Vec::new(),
            downloaded: 0,
            total: 0,
            logs: Vec::new(),
            status: TaskStatus::Running,
            error: None,
        }
    }

    /// 应用一条事件。
    pub fn apply(&mut self, event: &TaskEvent) {
        match event {
            TaskEvent::GraphStarted { steps, .. } => {
                self.steps = steps.clone();
                self.counts = vec![StepCounts::default(); steps.len()];
                self.downloaded = 0;
                self.total = 0;
            }
            TaskEvent::TaskAdded { task } => self.counts_mut(task).waiting += 1,
            TaskEvent::TaskStarted { task } => {
                let counts = self.counts_mut(task);
                counts.waiting = counts.waiting.saturating_sub(1);
                counts.running += 1;
            }
            TaskEvent::TaskFinished { task } => {
                let counts = self.counts_mut(task);
                counts.running = counts.running.saturating_sub(1);
                counts.completed += 1;
            }
            TaskEvent::TaskFailed { task } => {
                let counts = self.counts_mut(task);
                counts.running = counts.running.saturating_sub(1);
                counts.completed += 1;
                counts.failed += 1;
                if self.status != TaskStatus::Canceling {
                    self.status = TaskStatus::Failed;
                }
            }
            TaskEvent::TaskSkipped { task } => {
                let counts = self.counts_mut(task);
                counts.waiting = counts.waiting.saturating_sub(1);
                // 从未启动的任务不算完成，只记为「未执行」。
                counts.skipped += 1;
            }
            TaskEvent::Progress { downloaded, total } => {
                self.downloaded = *downloaded;
                self.total = *total;
            }
            TaskEvent::Log { message, .. } => self.logs.push(message.clone()),
            TaskEvent::Warning { message, .. } => self.logs.push(format!("⚠ 警告：{message}")),
            TaskEvent::GraphFinished | TaskEvent::GraphFailed => {}
        }
    }

    fn counts_mut(&mut self, task: &TaskRef) -> &mut StepCounts {
        let index = self
            .steps
            .iter()
            .position(|step| step.id == task.step())
            .unwrap_or(0);
        if self.counts.len() <= index {
            self.counts.resize(index + 1, StepCounts::default());
        }
        &mut self.counts[index]
    }

    /// 请求取消：正在运行时进入「取消中」。
    pub fn request_cancel(&mut self) {
        if self.status == TaskStatus::Running {
            self.status = TaskStatus::Canceling;
        }
    }

    /// 任务结束（`run` 的返回值）。
    pub fn finish(&mut self, ok: bool, cancelled: bool, error: Option<String>) {
        self.status = if cancelled {
            TaskStatus::Cancelled
        } else if ok {
            TaskStatus::Done
        } else {
            TaskStatus::Failed
        };
        // 取消也保留原因（通常是「任务已取消」或导致 FailFast 的首个错误）。
        self.error = if ok { None } else { error };
    }

    pub fn is_finished(&self) -> bool {
        matches!(
            self.status,
            TaskStatus::Done | TaskStatus::Cancelled | TaskStatus::Failed
        )
    }

    /// 进度条右侧的文字：只显示已下载 / 总字节。
    pub fn progress_text(&self) -> String {
        if self.total > 0 {
            format!(
                "{} / {}",
                human_bytes(self.downloaded),
                human_bytes(self.total)
            )
        } else if self.downloaded > 0 {
            human_bytes(self.downloaded)
        } else {
            String::new()
        }
    }

    /// 仍有任务没结束。
    pub fn tasks_remaining(&self) -> bool {
        self.counts.iter().any(StepCounts::active)
    }

    /// 确定进度值；不确定时返回 `None`（界面用 pulse）。
    pub fn global_fraction(&self) -> Option<f64> {
        if !self.tasks_remaining() || self.total == 0 || self.downloaded >= self.total {
            return None;
        }
        Some((self.downloaded as f64 / self.total as f64).clamp(0.0, 1.0))
    }

    /// 是否该用 pulse 当「不确定进度条」。
    pub fn wants_pulse(&self) -> bool {
        self.status == TaskStatus::Running
            && self.tasks_remaining()
            && self.global_fraction().is_none()
    }

    /// 每个 Step 的界面快照（按声明顺序）。
    pub fn step_views(&self) -> Vec<StepView> {
        let finished = self.is_finished();
        self.steps
            .iter()
            .enumerate()
            .map(|(index, step)| {
                let counts = self.counts.get(index).copied().unwrap_or_default();
                StepView::new(index + 1, step.name.clone(), counts, finished)
            })
            .collect()
    }

    /// 日志全文。
    pub fn logs_text(&self) -> String {
        self.logs.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dmcl_task::{task_fn, TaskGraphBuilder};
    use std::sync::{Arc, Mutex};

    #[tokio::test]
    async fn counts_and_step_discovery_follow_real_events() {
        let mut builder = TaskGraphBuilder::new("测试");
        let step = builder.step("准备");
        let first = builder.add(step, "一", (), task_fn(|_: (), _ctx| async move { Ok(()) }));
        let sink = builder.add(
            step,
            "二",
            first,
            task_fn(|_: Arc<()>, _ctx| async move { Ok(()) }),
        );
        let graph = builder.build(sink).unwrap();
        let events = Arc::new(Mutex::new(Vec::new()));
        let collected = events.clone();
        graph
            .run(move |event| collected.lock().unwrap().push(event))
            .await
            .unwrap();

        let mut state = TaskState::new("任务");
        for event in events.lock().unwrap().iter() {
            state.apply(event);
        }
        assert!(!state.tasks_remaining());
        assert_eq!(state.step_views().len(), 1);
        assert_eq!(state.step_views()[0].status, StepDisplayStatus::Done);
    }

    #[test]
    fn logs_and_global_progress_are_recorded() {
        let mut state = TaskState::new("t");
        state.apply(&TaskEvent::Log {
            task: None,
            message: "第一行".into(),
        });
        state.apply(&TaskEvent::Log {
            task: None,
            message: "第二行".into(),
        });
        assert_eq!(state.logs_text(), "第一行\n第二行");

        state.apply(&TaskEvent::Progress {
            downloaded: 2,
            total: 4,
        });
        assert_eq!(state.progress_text(), "2 B / 4 B");
        assert_eq!(state.global_fraction(), None);
    }

    #[test]
    fn step_rows_render_done_running_and_waiting() {
        let done = StepView::new(
            1,
            "数据准备".into(),
            StepCounts {
                completed: 12,
                ..Default::default()
            },
            false,
        );
        assert_eq!(done.icon_name(), "object-select-symbolic");
        assert_eq!(done.title(), "步骤 1  数据准备");
        assert_eq!(done.counts_text(), "12/12 完成");

        let running = StepView::new(
            3,
            "模型训练".into(),
            StepCounts {
                waiting: 3,
                running: 1,
                completed: 4,
                ..Default::default()
            },
            false,
        );
        assert_eq!(running.icon_name(), "content-loading-symbolic");
        assert_eq!(running.counts_text(), "4/8 运行中");

        let waiting = StepView::new(
            4,
            "评估".into(),
            StepCounts {
                waiting: 2,
                ..Default::default()
            },
            false,
        );
        assert_eq!(waiting.icon_name(), "radio-symbolic");
        assert_eq!(waiting.counts_text(), "0/2 等待中");
    }

    #[test]
    fn step_rows_render_skipped_tasks_as_not_run() {
        // 全部任务都没执行，或根本没有任务。
        let untouched = StepView::new(2, "安装".into(), StepCounts::default(), true);
        assert_eq!(untouched.status, StepDisplayStatus::Incomplete);
        assert_eq!(untouched.icon_name(), "radio-symbolic");
        assert_eq!(untouched.counts_text(), "未执行");

        // 部分完成、部分未执行：仍显示完成比例，但不算整步完成。
        let partial = StepView::new(
            3,
            "下载".into(),
            StepCounts {
                completed: 3,
                skipped: 1,
                ..Default::default()
            },
            true,
        );
        assert_eq!(partial.status, StepDisplayStatus::Incomplete);
        assert_eq!(partial.counts_text(), "3/4 完成");

        // 失败的 Step 只显示失败计数，不追加未执行数。
        let failed = StepView::new(
            5,
            "准备".into(),
            StepCounts {
                completed: 4,
                failed: 4,
                skipped: 1,
                ..Default::default()
            },
            true,
        );
        assert_eq!(failed.status, StepDisplayStatus::Failed);
        assert_eq!(failed.counts_text(), "4/5 失败");
    }

    #[tokio::test]
    async fn cancelled_waiting_tasks_are_not_counted_as_completed() {
        let mut builder = TaskGraphBuilder::new("取消测试");
        let prepare = builder.step("准备");
        let install = builder.step("安装");
        let blocker = builder.add(
            prepare,
            "阻塞",
            (),
            task_fn(|_: (), ctx| async move {
                ctx.cancel_handle().cancelled().await;
                Err(anyhow::anyhow!("任务已取消"))
            }),
        );
        let sink = builder.add(
            install,
            "后续",
            blocker,
            task_fn(|_: Arc<()>, _ctx| async move { Ok(()) }),
        );
        let graph = builder.build(sink).unwrap();
        let cancel = graph.cancel_handle();
        let trigger = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            cancel.cancel();
        });

        let events = Arc::new(Mutex::new(Vec::new()));
        let collected = events.clone();
        assert!(graph
            .run(move |event| collected.lock().unwrap().push(event))
            .await
            .is_err());
        trigger.await.unwrap();

        let mut state = TaskState::new("任务");
        for event in events.lock().unwrap().iter() {
            state.apply(event);
        }
        state.finish(false, true, Some("任务已取消".into()));

        let views = state.step_views();
        assert_eq!(views[0].status, StepDisplayStatus::Failed);
        assert_eq!(views[1].status, StepDisplayStatus::Incomplete);
        assert_eq!(views[1].completed, 0);
        assert_eq!(views[1].skipped, 1);
        assert_eq!(views[1].counts_text(), "未执行");
    }

    #[test]
    fn finish_sets_status_while_cancelling_is_kept() {
        let mut state = TaskState::new("t");
        state.request_cancel();
        assert_eq!(state.status, TaskStatus::Canceling);
        state.finish(false, true, Some("任务已取消".into()));
        assert_eq!(state.status, TaskStatus::Cancelled);
        assert_eq!(state.error.as_deref(), Some("任务已取消"));
        assert!(state.is_finished());

        state.finish(false, false, Some("boom".into()));
        assert_eq!(state.status, TaskStatus::Failed);
        assert_eq!(state.error.as_deref(), Some("boom"));
    }
}
