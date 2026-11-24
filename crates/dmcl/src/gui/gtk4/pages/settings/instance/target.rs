//! 把游戏设置表单绑定到某个实例的 `dmcl.json` 覆盖层。
//!
//! 表单只提交字段级改动：`fields` 写进实例文件，`inherit` 从实例文件删掉。实例文件不适合
//! 走全局配置仓库，所以这里直接异步写盘，并用一份乐观状态遮住「已经提交但还没写完」的
//! 窗口期。

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use serde_json::{Map, Value};

use crate::config::{instance_config_path, write_instance_launch};
use crate::gui::gtk4::app::{ConfigRef, PageToast, Shell};
use crate::gui::gtk4::pages::settings::{
    copy_fields, GameOptionField, GameOptionsState, GameOptionsTarget, GameOptionsUpdate,
};

/// 实例启动设置的落点。
pub(crate) struct InstanceGameOptionsTarget {
    config: ConfigRef,
    shell: Rc<Shell>,
    page_toast: PageToast,
    root: PathBuf,
    name: RefCell<String>,
    /// 写入还没落盘时的乐观状态；`pending > 0` 时优先返回它。
    optimistic: Rc<RefCell<Option<GameOptionsState>>>,
    /// 还没结束的写入数量。
    pending: Rc<Cell<u32>>,
    /// 串行化对同一个实例文件的写入。
    write_lock: Rc<tokio::sync::Mutex<()>>,
}

impl InstanceGameOptionsTarget {
    pub(crate) fn new(
        config: ConfigRef,
        shell: Rc<Shell>,
        page_toast: PageToast,
        root: PathBuf,
        name: String,
    ) -> Self {
        Self {
            config,
            shell,
            page_toast,
            root,
            name: RefCell::new(name),
            optimistic: Rc::new(RefCell::new(None)),
            pending: Rc::new(Cell::new(0)),
            write_lock: Rc::new(tokio::sync::Mutex::new(())),
        }
    }

    fn read_state(&self) -> GameOptionsState {
        let launch = self
            .config
            .read()
            .load_instance_sync(&self.root, self.name.borrow().as_str())
            .launch;
        GameOptionsState {
            launch,
            overrides: read_overrides(&self.root, self.name.borrow().as_str()),
        }
    }

    /// 实例改名后把后续读写指向新目录。
    pub(crate) fn rename(&self, name: &str) {
        *self.name.borrow_mut() = name.to_string();
    }

    fn submit(&self, update: GameOptionsUpdate) {
        let state = self.optimistic_state(&update);
        *self.optimistic.borrow_mut() = Some(state);

        let mut set = Map::new();
        let serialized = serde_json::to_value(&update.launch).expect("LaunchConfig 总能序列化");
        for field in &update.fields {
            if let Some(value) = serialized.get(field.key()) {
                set.insert(field.key().to_string(), value.clone());
            }
        }
        let remove: Vec<&str> = update.inherit.iter().map(|field| field.key()).collect();

        self.pending.set(self.pending.get() + 1);
        let root = self.root.clone();
        let name = self.name.borrow().clone();
        let shell = Rc::clone(&self.shell);
        let page_toast = self.page_toast.clone();
        let optimistic = Rc::clone(&self.optimistic);
        let pending = Rc::clone(&self.pending);
        let write_lock = Rc::clone(&self.write_lock);
        relm4::spawn_local(async move {
            let _writing = write_lock.lock().await;
            let result = write_instance_launch(&root, &name, set, &remove).await;
            let left = pending.get().saturating_sub(1);
            pending.set(left);
            match result {
                Ok(()) if left > 0 => {}
                Ok(()) => *optimistic.borrow_mut() = None,
                Err(err) => {
                    *optimistic.borrow_mut() = None;
                    pending.set(0);
                    shell.page_toast(&page_toast, &format!("保存实例配置失败：{err:#}"));
                }
            }
        });
    }

    /// 把这次改动叠加到当前状态上，得到写盘期间展示给表单的值。
    fn optimistic_state(&self, update: &GameOptionsUpdate) -> GameOptionsState {
        let mut state = self.load();
        copy_fields(&mut state.launch, &update.launch, &update.fields);
        let global = self.config.read().launch.clone();
        copy_fields(&mut state.launch, &global, &update.inherit);
        for field in &update.fields {
            state.overrides.insert(*field);
        }
        for field in &update.inherit {
            state.overrides.remove(field);
        }
        state
    }
}

impl GameOptionsTarget for InstanceGameOptionsTarget {
    fn load(&self) -> GameOptionsState {
        if self.pending.get() > 0 {
            if let Some(state) = self.optimistic.borrow().as_ref() {
                return state.clone();
            }
        }
        let state = self.read_state();
        *self.optimistic.borrow_mut() = Some(state.clone());
        state
    }

    fn save(&self, update: GameOptionsUpdate) {
        self.submit(update);
    }
}

/// 读实例文件里 `launch` 段出现过的字段；坏掉的字段也算覆盖，用户可以用「使用全局配置」删掉。
fn read_overrides(root: &Path, name: &str) -> BTreeSet<GameOptionField> {
    let path = instance_config_path(root, name);
    let Ok(text) = std::fs::read_to_string(path) else {
        return BTreeSet::new();
    };
    let Ok(value) = serde_json::from_str::<Value>(&text) else {
        return BTreeSet::new();
    };
    value
        .get("launch")
        .and_then(Value::as_object)
        .map(|launch| {
            launch
                .keys()
                .filter_map(|key| GameOptionField::from_key(key))
                .collect()
        })
        .unwrap_or_default()
}
