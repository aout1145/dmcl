//! 需要前端决策的警告。

use std::fmt;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

use tokio::sync::Notify;

const PENDING: u8 = 0;
const ACCEPTED: u8 = 1;
const ABORTED: u8 = 2;

/// 警告的"闸门"：任务在 [`WarningGate::wait`] 上等，前端用 `accept` / `abort` 放行。
pub(crate) struct WarningGate {
    state: AtomicU8,
    notify: Notify,
}

impl WarningGate {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            state: AtomicU8::new(PENDING),
            notify: Notify::new(),
        })
    }

    pub(crate) fn accept(&self) {
        self.resolve(ACCEPTED);
    }

    pub(crate) fn abort(&self) {
        self.resolve(ABORTED);
    }

    fn resolve(&self, state: u8) {
        // 只认第一次答复；用 `notify_one` 存下一个许可，避免与 `wait` 的注册赛跑。
        if self
            .state
            .compare_exchange(PENDING, state, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            self.notify.notify_one();
        }
    }

    /// 等答复：`true` = 继续，`false` = 中止。
    pub(crate) async fn wait(&self) -> bool {
        loop {
            match self.state.load(Ordering::SeqCst) {
                ACCEPTED => return true,
                ABORTED => return false,
                _ => self.notify.notified().await,
            }
        }
    }
}

/// 前端用来答复一条 [`crate::TaskEvent::Warning`] 的句柄：继续，或中止（中止 = 取消）。
///
/// 由任务随警告事件一起发出；前端 [`WarningDecision::accept`] / [`WarningDecision::abort`]
/// 之后任务才会继续。
#[derive(Clone)]
pub struct WarningDecision(pub(crate) Arc<WarningGate>);

impl WarningDecision {
    /// 继续执行。
    pub fn accept(&self) {
        self.0.accept();
    }

    /// 中止：任务会被取消（相当于触发 [`crate::CancelHandle::cancel`]）。
    pub fn abort(&self) {
        self.0.abort();
    }
}

impl fmt::Debug for WarningDecision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("WarningDecision")
    }
}

impl PartialEq for WarningDecision {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for WarningDecision {}
