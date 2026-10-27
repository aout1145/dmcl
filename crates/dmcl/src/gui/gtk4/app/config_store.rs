//! GTK4 前端的共享配置。
//!
//! 整个 GUI 只有一份 [`Config`]：GUI 入口用 [`init`] 把它放进进程级单例 [`ConfigStore`]，
//! 页面用 [`ConfigStore::config`] 拿只读句柄，写只能走 [`ConfigStore::update`]。
//!
//! 变更通知只有一个订阅者——根组件。单例里存一个 `tokio::sync::watch` 版本号，根组件在
//! GLib 主线程上把它转成自己的消息，负责补建新 root 的主页、刷新菜单并通知页面重画。
//!
//! [`ConfigStore::update`] 原地修改配置、通知订阅者，并在 tokio 上异步落盘；保存失败时
//! 回到 GLib 主线程调用调用方给的处理函数。落盘是 `update` 的一部分，调用方没有「忘记
//! 保存」或「忘记处理错误」的入口。
//!
//! 读 guard 要短命：`std::sync::RwLock` 不可重入，持着 guard 调 `update` 会自锁。用
//! `#[watch]` 读配置时一条语句内读完，不要让 guard 活到会写配置的回调里。

use std::sync::{Arc, OnceLock, RwLock, RwLockReadGuard};

use tokio::sync::watch;

use crate::config::Config;

/// 进程级单例。字段都是 `Send + Sync`，所以能放进 `static`。
static STORE: OnceLock<ConfigStore> = OnceLock::new();

/// 用启动时读到的配置初始化全局仓库，每个进程只调用一次。
pub(crate) fn init(config: Config) {
    if STORE.set(ConfigStore::new(config)).is_err() {
        panic!("配置仓库被重复初始化");
    }
}

/// 全局配置仓库；组件需要用配置时直接取用，不再逐层传参。
pub(crate) fn store() -> &'static ConfigStore {
    STORE
        .get()
        .expect("配置仓库尚未初始化：GUI 入口要先调用 config_store::init")
}

/// 配置的只读句柄。克隆只是复制共享指针，读到的永远是当前那份配置。
#[derive(Clone)]
pub(crate) struct ConfigRef(Arc<RwLock<Config>>);

impl ConfigRef {
    /// 只读访问当前配置。
    pub fn read(&self) -> RwLockReadGuard<'_, Config> {
        self.0.read().expect("配置锁已中毒")
    }
}

/// 落盘方式：普通保存，或先备份原文件再保存。
#[derive(Clone, Copy)]
enum PersistMode {
    Plain,
    WithBackup,
}

pub(crate) struct ConfigStore {
    config: Arc<RwLock<Config>>,
    /// 每次修改加一；根组件监听它。
    changes: watch::Sender<u64>,
    /// 串行化落盘；和配置锁分开，避免写文件时挡住读配置。
    save_lock: Arc<tokio::sync::Mutex<()>>,
}

impl ConfigStore {
    fn new(config: Config) -> Self {
        Self {
            config: Arc::new(RwLock::new(config)),
            changes: watch::channel(0).0,
            save_lock: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    /// 配置的只读句柄。
    pub fn config(&self) -> ConfigRef {
        ConfigRef(Arc::clone(&self.config))
    }

    /// 变更通知；只有根组件订阅，由它把变更分发给各页面。
    pub fn changes(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }

    /// 原地修改配置、通知订阅者，并在 tokio 上异步落盘。
    ///
    /// 保存失败（或保存任务意外结束）时，回到 GLib 主线程调用 `on_error`。
    /// 必须在 GTK 主线程调用：`on_error` 直接用于弹框 / 发消息。
    pub fn update(
        &self,
        f: impl FnOnce(&mut Config),
        on_error: impl FnOnce(anyhow::Error) + 'static,
    ) {
        self.apply(f);
        self.persist(PersistMode::Plain, move |result| {
            if let Err(err) = result {
                on_error(err);
            }
        });
    }

    /// 写入修复后的配置，并先把原文件备份为同目录的 `dmcl.old.json`。
    ///
    /// 保存结束后回到 GLib 主线程调用 `on_done`。必须在 GTK 主线程调用。
    pub fn save_with_backup(&self, on_done: impl FnOnce(anyhow::Result<()>) + 'static) {
        self.persist(PersistMode::WithBackup, on_done);
    }

    fn persist(&self, mode: PersistMode, on_done: impl FnOnce(anyhow::Result<()>) + 'static) {
        let config = Arc::clone(&self.config);
        let save_lock = Arc::clone(&self.save_lock);
        let (tx, rx) = tokio::sync::oneshot::channel();
        relm4::spawn(async move {
            let _saving = save_lock.lock().await;
            // 写盘不能跨 `await` 持读锁：先 clone 一份序列化用的快照。共享的那份配置
            // 没有变，这里只是给异步写文件用的临时副本。
            let snapshot = config.read().expect("配置锁已中毒").clone();
            let result = match mode {
                PersistMode::Plain => snapshot.save().await,
                PersistMode::WithBackup => snapshot.save_with_backup().await,
            };
            let _ = tx.send(result);
        });
        relm4::spawn_local(async move {
            match rx.await {
                Ok(result) => on_done(result),
                // 保存任务在回传结果之前结束：不能静默当成成功。
                Err(_) => on_done(Err(anyhow::anyhow!("保存配置任务意外结束"))),
            }
        });
    }

    /// 只改内存并通知，不落盘。
    fn apply(&self, f: impl FnOnce(&mut Config)) {
        {
            let mut config = self.config.write().expect("配置锁已中毒");
            f(&mut config);
        }
        // 连续修改只推进版本号；监听者读到的是最新配置，不需要逐条处理。
        self.changes.send_modify(|version| *version += 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 测试只走 apply：`update` 会真的写 `Config::path()`，不能碰用户的配置。

    #[test]
    fn apply_changes_the_config_seen_through_existing_handles() {
        let store = ConfigStore::new(Config::default());
        let config = store.config();

        let before = config.read().launch.max_memory;
        store.apply(|config| config.launch.max_memory = before + 1);
        assert_eq!(config.read().launch.max_memory, before + 1);
    }

    #[test]
    fn watching_starts_clean_and_apply_bumps_the_version() {
        let store = ConfigStore::new(Config::default());
        let mut changes = store.changes();
        assert!(!changes.has_changed().unwrap(), "订阅时不应该补发历史变更");

        store.apply(|_| ());
        assert!(changes.has_changed().unwrap());
        changes.mark_unchanged();
        assert!(!changes.has_changed().unwrap());
    }
}
