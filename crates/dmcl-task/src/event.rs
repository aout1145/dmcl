//! 任务图对外的事件与任务引用。

use std::fmt;
use std::sync::Arc;

use crate::graph::{StepDef, StepId};
use crate::warning::WarningDecision;

/// 一个任务节点在某一时刻的状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TaskStatus {
    /// 已加入图，等待依赖完成。
    Waiting,
    /// 正在执行（包括等待动态子节点 / 警告答复）。
    Running,
    /// 成功完成。
    Finished,
    /// 失败或被取消。
    Failed,
    /// 依赖失败 / 全局取消，没有开始执行。
    Skipped,
}

impl TaskStatus {
    pub(crate) fn as_u8(self) -> u8 {
        match self {
            TaskStatus::Waiting => 0,
            TaskStatus::Running => 1,
            TaskStatus::Finished => 2,
            TaskStatus::Failed => 3,
            TaskStatus::Skipped => 4,
        }
    }

    pub(crate) fn from_u8(value: u8) -> Self {
        match value {
            1 => TaskStatus::Running,
            2 => TaskStatus::Finished,
            3 => TaskStatus::Failed,
            4 => TaskStatus::Skipped,
            _ => TaskStatus::Waiting,
        }
    }
}

/// 事件里对某个任务节点的引用。
///
/// 内部带一个图内唯一的序号用于 `Hash` / `Debug`，但它不是公开的节点 ID：外部只能
/// 通过 [`TaskRef::step`] / [`TaskRef::name`] / [`TaskRef::status`] 使用它。
#[derive(Clone)]
pub struct TaskRef {
    id: usize,
    step: StepId,
    name: Arc<str>,
    status: TaskStatus,
}

impl TaskRef {
    pub(crate) fn new(id: usize, step: StepId, name: Arc<str>, status: TaskStatus) -> Self {
        Self {
            id,
            step,
            name,
            status,
        }
    }

    /// 所属的显示步骤。
    pub fn step(&self) -> StepId {
        self.step
    }

    /// 显示名。
    pub fn name(&self) -> &str {
        &self.name
    }

    /// 事件发生时的状态。
    pub fn status(&self) -> TaskStatus {
        self.status
    }
}

impl fmt::Debug for TaskRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TaskRef")
            .field("id", &self.id)
            .field("step", &self.step)
            .field("name", &self.name)
            .field("status", &self.status)
            .finish()
    }
}

impl fmt::Display for TaskRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name)
    }
}

impl PartialEq for TaskRef {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for TaskRef {}

impl std::hash::Hash for TaskRef {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

/// 任务图运行过程中向订阅者推送的事件。
///
/// 事件是**只进不退**的单向流：图结束后事件总线关闭，订阅者排空事件后
/// [`crate::EventReceiver::recv`] 返回 `None`。
#[derive(Debug, Clone, PartialEq)]
pub enum TaskEvent {
    /// 图开始运行，一次性带出全部静态 Step。
    GraphStarted { name: String, steps: Vec<StepDef> },

    /// 一个节点已加入图（初始为等待中）。
    TaskAdded { task: TaskRef },
    /// 一个节点开始执行。
    TaskStarted { task: TaskRef },
    /// 一个节点成功完成。
    TaskFinished { task: TaskRef },
    /// 一个节点失败或被取消。
    TaskFailed { task: TaskRef },
    /// 一个节点依赖失败 / 全局取消，没有执行。
    TaskSkipped { task: TaskRef },

    /// 全局下载进度快照。
    Progress { downloaded: u64, total: u64 },

    /// 一条一次性日志。
    Log {
        task: Option<TaskRef>,
        message: String,
    },

    /// 一条需要前端决策的警告；当前任务会阻塞等待答复。
    Warning {
        task: Option<TaskRef>,
        message: String,
        decision: WarningDecision,
    },

    /// 整张图成功结束。
    GraphFinished,
    /// 整张图失败 / 被取消。
    GraphFailed,
}

impl TaskEvent {
    /// 事件关联的任务（图级事件为 `None`）。
    pub fn task(&self) -> Option<&TaskRef> {
        match self {
            TaskEvent::TaskAdded { task }
            | TaskEvent::TaskStarted { task }
            | TaskEvent::TaskFinished { task }
            | TaskEvent::TaskFailed { task }
            | TaskEvent::TaskSkipped { task } => Some(task),
            TaskEvent::Log { task, .. } | TaskEvent::Warning { task, .. } => task.as_ref(),
            TaskEvent::GraphStarted { .. }
            | TaskEvent::Progress { .. }
            | TaskEvent::GraphFinished
            | TaskEvent::GraphFailed => None,
        }
    }
}

impl fmt::Display for TaskEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TaskEvent::GraphStarted { name, steps } => {
                write!(f, "{name}: 开始（{} 个步骤）", steps.len())
            }
            TaskEvent::TaskAdded { task } => write!(f, "{}: 等待", task.name()),
            TaskEvent::TaskStarted { task } => write!(f, "{}: 执行中", task.name()),
            TaskEvent::TaskFinished { task } => write!(f, "{}: 完成", task.name()),
            TaskEvent::TaskFailed { task } => write!(f, "{}: 失败", task.name()),
            TaskEvent::TaskSkipped { task } => write!(f, "{}: 跳过", task.name()),
            TaskEvent::Progress { downloaded, total } => write!(
                f,
                "进度: {} / {}",
                crate::progress::human_bytes(*downloaded),
                crate::progress::human_bytes(*total)
            ),
            TaskEvent::Log { task, message } => match task {
                Some(task) => write!(f, "{}: {message}", task.name()),
                None => f.write_str(message),
            },
            TaskEvent::Warning { task, message, .. } => match task {
                Some(task) => write!(f, "{}: 警告: {message}", task.name()),
                None => write!(f, "警告: {message}"),
            },
            TaskEvent::GraphFinished => f.write_str("全部完成"),
            TaskEvent::GraphFailed => f.write_str("已中止"),
        }
    }
}
