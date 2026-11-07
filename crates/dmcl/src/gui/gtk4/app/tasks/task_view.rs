//! 任务卡片的状态与进度归约（纯逻辑，可单测）。
//!
//! 图的 Step 是静态的：`GraphStarted` 一次性带出全部 Step，之后按任务事件累计每个 Step
//! 的等待 / 执行 / 完成计数，并记录每个任务的名字与状态（展开 Step 时显示）；
//! 全局进度只有 `downloaded / total`。

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

/// 一个任务在展开的 Step 里的状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum TaskItemStatus {
    Waiting,
    Running,
    Done,
    Failed,
    /// 依赖失败 / 全局取消，没有执行。
    Skipped,
}

/// 一个任务在展开的 Step 里的一行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TaskItemView {
    pub name: String,
    pub status: TaskItemStatus,
}

impl TaskItemView {
    pub fn status_text(&self) -> &'static str {
        match self.status {
            TaskItemStatus::Waiting => "等待中",
            TaskItemStatus::Running => "运行中",
            TaskItemStatus::Done => "完成",
            TaskItemStatus::Failed => "失败",
            TaskItemStatus::Skipped => "未执行",
        }
    }

    /// 状态文字的样式：`caption` 加上按状态区分的颜色。
    pub fn status_classes(&self) -> Vec<&'static str> {
        let mut classes = vec!["caption"];
        classes.extend(match self.status {
            TaskItemStatus::Done => vec!["success"],
            TaskItemStatus::Running => vec!["accent"],
            TaskItemStatus::Waiting | TaskItemStatus::Skipped => vec!["dim-label"],
            TaskItemStatus::Failed => vec!["error"],
        });
        classes
    }
}

/// 内部跟踪的任务：身份用于去重，状态用于显示。
#[derive(Debug, Clone)]
struct TrackedTask {
    task: TaskRef,
    status: TaskItemStatus,
}

/// 展开 Step 时最多渲染的任务行数。下载会为每个文件创建节点，几千行会拖垮 UI；
/// 其余任务只在底部显示数量。
const MAX_VISIBLE_TASKS: usize = 50;

/// 任务行的显示优先级：正在进行的排最前，其次是等待中，其余（完成 / 失败 / 未执行）排最后。
fn status_rank(status: TaskItemStatus) -> u8 {
    match status {
        TaskItemStatus::Running => 0,
        TaskItemStatus::Waiting => 1,
        TaskItemStatus::Done | TaskItemStatus::Failed | TaskItemStatus::Skipped => 2,
    }
}

/// 按优先级挑出最多 `limit` 个任务下标；同一优先级内保持加入顺序。
fn visible_indices(statuses: &[TaskItemStatus], limit: usize) -> Vec<usize> {
    if limit == 0 {
        return Vec::new();
    }
    let mut indices = Vec::with_capacity(limit.min(statuses.len()));
    for rank in 0..=2 {
        for (index, status) in statuses.iter().enumerate() {
            if status_rank(*status) == rank {
                indices.push(index);
                if indices.len() == limit {
                    return indices;
                }
            }
        }
    }
    indices
}

/// 生成展开区要显示的任务快照与未显示数量。
fn visible_tasks(tracked: &[TrackedTask]) -> (Vec<TaskItemView>, u64) {
    let statuses: Vec<_> = tracked.iter().map(|item| item.status).collect();
    let indices = visible_indices(&statuses, MAX_VISIBLE_TASKS);
    let hidden = (tracked.len() - indices.len()) as u64;
    let tasks = indices
        .into_iter()
        .map(|index| TaskItemView {
            name: tracked[index].task.name().to_string(),
            status: tracked[index].status,
        })
        .collect();
    (tasks, hidden)
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
    /// 展开时显示的任务明细：运行中 → 等待中 → 其余，最多 [`MAX_VISIBLE_TASKS`] 条。
    pub tasks: Vec<TaskItemView>,
    /// 因为超过上限而没有放进 `tasks` 的任务数。
    pub hidden_tasks: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StepDisplayStatus {
    Waiting,
    Running,
    Done,
    /// 图结束后仍有任务没执行（取消 / 失败时停在等待），或者整个 Step 没有任何任务。
    Incomplete,
    Failed,
}

impl StepView {
    fn new(
        number: usize,
        name: String,
        counts: StepCounts,
        tasks: Vec<TaskItemView>,
        hidden_tasks: u64,
        finished: bool,
    ) -> Self {
        // 取消 / 失败会让调度器立刻把未开始的节点标成 Skipped，但正在跑的节点还要收尾；
        // 图真正结束前先按「等待中」显示，避免一点取消就把这些 Step 标成「未完成」。
        let incomplete = counts.waiting > 0 || counts.skipped > 0 || counts.total() == 0;
        let status = if counts.running > 0 {
            StepDisplayStatus::Running
        } else if counts.failed > 0 {
            StepDisplayStatus::Failed
        } else if incomplete {
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
            tasks,
            hidden_tasks,
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

    /// 右侧计数的样式：`caption` 控制字号，再叠加与状态图标一致的颜色。
    pub fn counts_classes(&self) -> Vec<&'static str> {
        let mut classes = vec!["caption"];
        classes.extend(self.icon_classes());
        classes
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
    /// 与 `steps` 对齐的任务明细，展开 Step 时显示。
    tasks: Vec<Vec<TrackedTask>>,
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
            tasks: Vec::new(),
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
                self.tasks = vec![Vec::new(); steps.len()];
                self.downloaded = 0;
                self.total = 0;
            }
            TaskEvent::TaskAdded { task } => {
                self.counts_mut(task).waiting += 1;
                self.record(task, TaskItemStatus::Waiting);
            }
            TaskEvent::TaskStarted { task } => {
                let counts = self.counts_mut(task);
                counts.waiting = counts.waiting.saturating_sub(1);
                counts.running += 1;
                self.record(task, TaskItemStatus::Running);
            }
            TaskEvent::TaskFinished { task } => {
                let counts = self.counts_mut(task);
                counts.running = counts.running.saturating_sub(1);
                counts.completed += 1;
                self.record(task, TaskItemStatus::Done);
            }
            TaskEvent::TaskFailed { task } => {
                let counts = self.counts_mut(task);
                counts.running = counts.running.saturating_sub(1);
                counts.completed += 1;
                counts.failed += 1;
                if self.status != TaskStatus::Canceling {
                    self.status = TaskStatus::Failed;
                }
                self.record(task, TaskItemStatus::Failed);
            }
            TaskEvent::TaskSkipped { task } => {
                let counts = self.counts_mut(task);
                counts.waiting = counts.waiting.saturating_sub(1);
                // 从未启动的任务不算完成，只记为「未执行」。
                counts.skipped += 1;
                self.record(task, TaskItemStatus::Skipped);
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

    /// 记录 / 更新一个任务的显示状态。
    fn record(&mut self, task: &TaskRef, status: TaskItemStatus) {
        let index = self
            .steps
            .iter()
            .position(|step| step.id == task.step())
            .unwrap_or(0);
        if self.tasks.len() <= index {
            self.tasks.resize_with(index + 1, Vec::new);
        }
        let tasks = &mut self.tasks[index];
        match tasks.iter_mut().find(|tracked| tracked.task == *task) {
            Some(tracked) => tracked.status = status,
            None => tasks.push(TrackedTask {
                task: task.clone(),
                status,
            }),
        }
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
    ///
    /// 取消中始终返回 `None`：收尾阶段不再关心百分比，进度条保持不确定动画。
    pub fn global_fraction(&self) -> Option<f64> {
        if self.status == TaskStatus::Canceling
            || !self.tasks_remaining()
            || self.total == 0
            || self.downloaded >= self.total
        {
            return None;
        }
        Some((self.downloaded as f64 / self.total as f64).clamp(0.0, 1.0))
    }

    /// 是否该用 pulse 当「不确定进度条」。
    pub fn wants_pulse(&self) -> bool {
        match self.status {
            // 取消中不管之前的进度是多少，都保持不确定动画，直到图真正结束。
            TaskStatus::Canceling => true,
            TaskStatus::Running => self.tasks_remaining() && self.global_fraction().is_none(),
            TaskStatus::Done | TaskStatus::Cancelled | TaskStatus::Failed => false,
        }
    }

    /// 每个 Step 的界面快照（按声明顺序）。
    pub fn step_views(&self) -> Vec<StepView> {
        let finished = self.is_finished();
        self.steps
            .iter()
            .enumerate()
            .map(|(index, step)| {
                let counts = self.counts.get(index).copied().unwrap_or_default();
                let tracked = self.tasks.get(index).map(Vec::as_slice).unwrap_or(&[]);
                let (tasks, hidden_tasks) = visible_tasks(tracked);
                StepView::new(
                    index + 1,
                    step.name.clone(),
                    counts,
                    tasks,
                    hidden_tasks,
                    finished,
                )
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
        let views = state.step_views();
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].status, StepDisplayStatus::Done);

        // 每个任务的名字与最终状态都保留下来，供展开 Step 时显示。
        let tasks = &views[0].tasks;
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].name, "一");
        assert_eq!(tasks[0].status, TaskItemStatus::Done);
        assert_eq!(tasks[1].name, "二");
        assert_eq!(tasks[1].status, TaskItemStatus::Done);
    }

    #[test]
    fn task_items_render_each_state() {
        let item = |status| TaskItemView {
            name: "下载".into(),
            status,
        };
        assert_eq!(item(TaskItemStatus::Waiting).status_text(), "等待中");
        assert_eq!(item(TaskItemStatus::Running).status_text(), "运行中");
        assert_eq!(item(TaskItemStatus::Done).status_text(), "完成");
        assert_eq!(item(TaskItemStatus::Failed).status_text(), "失败");
        assert_eq!(item(TaskItemStatus::Skipped).status_text(), "未执行");
        assert_eq!(
            item(TaskItemStatus::Failed).status_classes(),
            vec!["caption", "error"]
        );
    }

    #[test]
    fn visible_tasks_put_running_then_waiting_then_finished_first() {
        use TaskItemStatus::*;
        let statuses = [
            Done, Waiting, Running, Skipped, Done, Waiting, Running, Failed,
        ];
        assert_eq!(visible_indices(&statuses, 4), vec![2, 6, 1, 5]);
        assert_eq!(
            visible_indices(&statuses, 100),
            vec![2, 6, 1, 5, 0, 3, 4, 7]
        );
        assert!(visible_indices(&statuses, 0).is_empty());

        // 运行中的任务不会因为有大量已完成任务而被截掉。
        let mut many = vec![Done; MAX_VISIBLE_TASKS];
        many[MAX_VISIBLE_TASKS - 1] = Running;
        let picked = visible_indices(&many, 5);
        assert_eq!(picked.len(), 5);
        assert_eq!(picked[0], MAX_VISIBLE_TASKS - 1);
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
    fn canceling_always_keeps_the_progress_bar_indeterminate() {
        let mut state = TaskState::new("t");
        state.counts = vec![StepCounts {
            running: 1,
            ..Default::default()
        }];
        state.total = 100;
        state.downloaded = 50;
        assert_eq!(state.global_fraction(), Some(0.5));
        assert!(!state.wants_pulse());

        state.request_cancel();
        assert_eq!(state.global_fraction(), None);
        assert!(state.wants_pulse());
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
            Vec::new(),
            0,
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
            Vec::new(),
            0,
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
            Vec::new(),
            0,
            false,
        );
        assert_eq!(waiting.icon_name(), "radio-symbolic");
        assert_eq!(waiting.counts_text(), "0/2 等待中");
    }

    #[test]
    fn step_rows_render_skipped_tasks_as_not_run() {
        // 全部任务都没执行，或根本没有任务。
        let untouched = StepView::new(2, "安装".into(), StepCounts::default(), Vec::new(), 0, true);
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
            Vec::new(),
            0,
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
            Vec::new(),
            0,
            true,
        );
        assert_eq!(failed.status, StepDisplayStatus::Failed);
        assert_eq!(failed.counts_text(), "4/5 失败");
    }

    #[test]
    fn skipped_tasks_become_incomplete_only_after_the_graph_finishes() {
        // 点取消后正在跑的任务还要收尾；这期间未开始的 Step 仍按等待中显示。
        let untouched = StepCounts {
            skipped: 2,
            ..Default::default()
        };
        assert_eq!(
            StepView::new(1, "安装".into(), untouched, Vec::new(), 0, false).status,
            StepDisplayStatus::Waiting
        );
        assert_eq!(
            StepView::new(1, "安装".into(), untouched, Vec::new(), 0, true).status,
            StepDisplayStatus::Incomplete
        );

        let partial = StepCounts {
            completed: 3,
            skipped: 1,
            ..Default::default()
        };
        assert_eq!(
            StepView::new(1, "下载".into(), partial, Vec::new(), 0, false).status,
            StepDisplayStatus::Waiting
        );
        assert_eq!(
            StepView::new(1, "下载".into(), partial, Vec::new(), 0, true).status,
            StepDisplayStatus::Incomplete
        );
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
