//! 进度事件流。

use std::fmt;

use crate::progress::Progress;

/// 任务链在运行过程中向订阅者推送的事件。
///
/// 事件是**只进不退**的单向流：链运行结束后发送方被丢弃，订阅者的 `recv()` 会返回
/// `Err`，循环自然结束。失败时先发一条 [`TaskEvent::Failed`] 再结束（链本身的错误
/// 仍由 `run()` 的返回值给出）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskEvent {
    /// 整条链开始运行。
    Started { name: String, steps: usize },
    /// 第 `step` 个步骤开始。
    StepStarted {
        step: usize,
        steps: usize,
        name: String,
    },
    /// 第 `step` 个步骤的一条**日志**：一次性的文字说明。
    ///
    /// 前端应把它当成"一行输出"逐条打印，**不要**像进度那样原地覆盖（例如命令行的
    /// 每一行输出、缺件清单的每一条）。同一步骤可以发很多条。
    Log {
        step: usize,
        steps: usize,
        name: String,
        message: String,
    },
    /// 第 `step` 个步骤的**进度**：带工作量的、可原地刷新的进度（同一个位置反复更新）。
    Progress {
        step: usize,
        steps: usize,
        name: String,
        progress: Progress,
    },
    /// 第 `step` 个步骤成功结束。
    StepFinished {
        step: usize,
        steps: usize,
        name: String,
    },
    /// 第 `step` 个步骤失败，链就此中止。
    ///
    /// 只带“哪一步失败了”的结构信息；**错误详情在 `run()` / `run_with()` 的返回值里**，
    /// 由调用者打印一次，避免事件流和返回值把同一条错误各打一遍。
    Failed {
        step: usize,
        steps: usize,
        name: String,
    },
    /// 整条链成功结束。
    Finished { name: String },
}

impl TaskEvent {
    /// 事件所属的名字（链名或步骤名）。
    pub fn name(&self) -> &str {
        match self {
            TaskEvent::Started { name, .. }
            | TaskEvent::StepStarted { name, .. }
            | TaskEvent::Log { name, .. }
            | TaskEvent::Progress { name, .. }
            | TaskEvent::StepFinished { name, .. }
            | TaskEvent::Failed { name, .. }
            | TaskEvent::Finished { name } => name,
        }
    }

    /// 事件对应的步骤下标（链级事件为 0）。
    pub fn step(&self) -> usize {
        match self {
            TaskEvent::Started { .. } | TaskEvent::Finished { .. } => 0,
            TaskEvent::StepStarted { step, .. }
            | TaskEvent::Log { step, .. }
            | TaskEvent::Progress { step, .. }
            | TaskEvent::StepFinished { step, .. }
            | TaskEvent::Failed { step, .. } => *step,
        }
    }

    /// 最新的进度（非进度事件为 `None`）。
    pub fn progress(&self) -> Option<&Progress> {
        match self {
            TaskEvent::Progress { progress, .. } => Some(progress),
            _ => None,
        }
    }
}

impl fmt::Display for TaskEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TaskEvent::Started { name, steps } => write!(f, "{name}: 开始（{steps} 个步骤）"),
            TaskEvent::StepStarted { step, steps, name } => {
                write!(f, "[{}/{steps}] {name}", step + 1)
            }
            TaskEvent::Log {
                step,
                steps,
                name,
                message,
            } => write!(f, "[{}/{steps}] {name}: {message}", step + 1),
            TaskEvent::Progress {
                step,
                steps,
                name,
                progress,
            } => write!(f, "[{}/{steps}] {name}: {progress}", step + 1),
            TaskEvent::StepFinished { step, steps, name } => {
                write!(f, "[{}/{steps}] {name}: 完成", step + 1)
            }
            TaskEvent::Failed { step, steps, name } => {
                write!(f, "[{}/{steps}] {name}: 失败", step + 1)
            }
            TaskEvent::Finished { name } => write!(f, "{name}: 全部完成"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_expose_their_name_and_step() {
        let started = TaskEvent::StepStarted {
            step: 2,
            steps: 5,
            name: "下载".to_string(),
        };
        assert_eq!(started.name(), "下载");
        assert_eq!(started.step(), 2);
        assert_eq!(started.to_string(), "[3/5] 下载");

        let progress = TaskEvent::Progress {
            step: 0,
            steps: 1,
            name: "x".to_string(),
            progress: Progress::new(1, Some(2), "f.jar"),
        };
        assert_eq!(progress.progress().unwrap().current, 1);
        assert_eq!(progress.to_string(), "[1/1] x: 50.0% (1 B / 2 B) f.jar");

        assert!(TaskEvent::Finished { name: "x".into() }
            .progress()
            .is_none());
    }
}
