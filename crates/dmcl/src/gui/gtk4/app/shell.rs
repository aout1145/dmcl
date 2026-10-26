//! Shell：应用级共享服务。
//!
//! 它是**通用**的「跑任务 + 公共 UI」服务，不知道实例/游戏是什么：
//!
//! * [`Shell::run`] 把一条任务链跑起来，把进度事件转发给任务 popup，并返回一个
//!   **可 `await` 的结果句柄**（不是回调）；
//! * [`Shell::open_tasks`] 主动打开任务列表窗口；
//! * [`Shell::global_toast`] / [`Shell::page_toast`] 在主界面弹提示，前者始终显示，
//!   后者只在页面可见时显示。

use std::cell::Cell;

use dmcl_task::{Cancelled, TaskChain};

use super::tasks::TaskMsg;
use super::toast::{PageToast, ToastService};

/// 任务结果句柄：`finished().await` 拿到任务产出。
pub(crate) struct TaskHandle<O> {
    rx: tokio::sync::oneshot::Receiver<Result<O, String>>,
}

impl<O> TaskHandle<O> {
    pub async fn finished(self) -> Result<O, String> {
        self.rx.await.unwrap_or_else(|_| Err("任务已结束".into()))
    }
}

/// 应用级共享服务。
pub(crate) struct Shell {
    /// 任务 popup 的入口（主线程）。
    tasks: relm4::Sender<TaskMsg>,
    /// 应用级提示服务。
    toasts: ToastService,
    next_id: Cell<u64>,
}

impl Shell {
    pub fn new(tasks: relm4::Sender<TaskMsg>, toasts: ToastService) -> Self {
        Self {
            tasks,
            toasts,
            next_id: Cell::new(0),
        }
    }

    /// 跑一个任务：事件喂给任务 popup，返回结果句柄。
    pub fn run<I, O>(&self, name: String, chain: TaskChain<I, O>, input: I) -> TaskHandle<O>
    where
        I: Send + 'static,
        O: Send + 'static,
    {
        let id = self.next_id.get();
        self.next_id.set(id + 1);

        let cancel = chain.cancel_handle();
        let _ = self.tasks.send(TaskMsg::Start { id, name, cancel });

        let (tx, rx) = tokio::sync::oneshot::channel();
        let events = self.tasks.clone();
        let done = self.tasks.clone();
        relm4::spawn(async move {
            let raw = chain
                .run_with(input, move |event| {
                    let _ = events.send(TaskMsg::Event { id, event });
                })
                .await;
            // 先认出「用户取消」，再转成字符串：取消不是失败，界面要显示「已取消」。
            let cancelled = raw.as_ref().err().is_some_and(|err| {
                err.chain()
                    .any(|cause| cause.downcast_ref::<Cancelled>().is_some())
            });
            let result = raw.map_err(|err| format!("{err:#}"));
            let _ = done.send(TaskMsg::Finished {
                id,
                ok: result.is_ok(),
                cancelled,
                error: result.as_ref().err().cloned(),
            });
            let _ = tx.send(result);
        });

        TaskHandle { rx }
    }

    /// 主动打开任务列表窗口（任务按钮走的是同一条 `TaskMsg::Open`）。
    pub fn open_tasks(&self) {
        let _ = self.tasks.send(TaskMsg::Open);
    }

    /// 应用级提示：始终显示，不依赖页面是否可见。
    pub fn global_toast(&self, text: &str) {
        self.toasts.global_toast(text);
    }

    /// 页面级提示：页面当前可见时才显示，否则直接丢弃。
    pub fn page_toast(&self, page: &PageToast, text: &str) {
        self.toasts.page_toast(page, text);
    }
}
