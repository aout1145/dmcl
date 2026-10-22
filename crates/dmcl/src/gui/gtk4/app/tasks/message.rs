//! 任务对话框的消息。

use dmcl_task::{CancelHandle, TaskEvent};
use relm4::prelude::DynamicIndex;

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
    /// 任务结束。`cancelled` 表示是用户取消的；`error` 是失败原因（成功 / 取消时为 `None`）。
    Finished {
        id: u64,
        ok: bool,
        cancelled: bool,
        error: Option<String>,
    },
    /// 卡片右上角的叉：运行中 = 取消，已结束 = 清除这条记录。
    CloseCard(DynamicIndex),
    /// 用户请求关闭任务窗口（窗口的 `close-request`）：只隐藏，不取消任务。
    CloseRequested,
    /// 用户请求关闭主窗口：有未结束任务时先确认。
    CloseMain,
    /// 确认退出：取消所有未结束的任务，然后通知 app 层退出。
    CancelAllAndQuit,
}
