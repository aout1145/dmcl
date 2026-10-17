//! Shell：应用级共享服务。
//!
//! 它是**通用**的「跑任务 + 公共 UI」服务，不知道实例/游戏是什么：
//!
//! * [`Shell::run`] 把一条任务链跑起来，把进度事件转发给任务 popup，并返回一个
//!   **可 `await` 的结果句柄**（不是回调）；
//! * [`Shell::toast`] 在主界面弹一行提示。

use std::cell::Cell;

use relm4::gtk;

use dmcl_task::TaskChain;

use super::task_popup::TaskMsg;
use super::toast;

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
    /// 主界面的提示层。
    toast_layer: gtk::Box,
    next_id: Cell<u64>,
}

impl Shell {
    pub fn new(tasks: relm4::Sender<TaskMsg>, toast_layer: gtk::Box) -> Self {
        Self {
            tasks,
            toast_layer,
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
            let result = chain
                .run_with(input, move |event| {
                    let _ = events.send(TaskMsg::Event { id, event });
                })
                .await
                .map_err(|err| format!("{err:#}"));
            let _ = done.send(TaskMsg::Finished {
                id,
                ok: result.is_ok(),
                error: result.as_ref().err().cloned(),
            });
            let _ = tx.send(result);
        });

        TaskHandle { rx }
    }

    /// 在主界面弹一行提示。
    pub fn toast(&self, text: &str) {
        toast::show(&self.toast_layer, text);
    }
}
