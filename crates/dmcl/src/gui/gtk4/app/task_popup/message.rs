//! 任务对话框的消息。

use dmcl_task::{CancelHandle, TaskEvent};

/// 任务对话框收到的消息。
#[derive(Debug)]
pub(crate) enum TaskMsg {
    /// 打开对话框。
    Open,
    /// 一个任务开始。
    Start {
        id: u64,
        name: String,
        cancel: CancelHandle,
    },
    /// 任务事件流里的一条。
    Event { id: u64, event: TaskEvent },
    /// 任务结束。`error` 是失败原因（成功时为 `None`）。
    Finished {
        id: u64,
        ok: bool,
        error: Option<String>,
    },
    /// 卡片右上角的叉：运行中 = 取消，已结束 = 清除这条记录。
    Close(u64),
    /// 清掉所有已经结束的任务（关闭对话框时用）。
    ClearFinished,
}
