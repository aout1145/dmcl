//! 把游戏设置表单绑定到全局共享配置。
//!
//! 表单只发字段级改动，这里负责把改动写进 `Config::launch` 并异步落盘；保存失败用首选项
//! 页的 toast 提示。实例管理页以后会实现另一个 `GameOptionsTarget`，表单本身不用改。

use std::rc::Rc;

use crate::config::LaunchConfig;
use crate::gui::gtk4::app::{config_store, ConfigRef, PageToast, Shell};

use super::{apply_update, GameOptionsTarget, GameOptionsUpdate};

/// 「全局游戏选项」分类的落点。
pub(crate) struct GlobalGameOptionsTarget {
    config: ConfigRef,
    shell: Rc<Shell>,
    page_toast: PageToast,
}

impl GlobalGameOptionsTarget {
    pub(crate) fn new(shell: Rc<Shell>, page_toast: PageToast, config: ConfigRef) -> Self {
        Self {
            config,
            shell,
            page_toast,
        }
    }
}

impl GameOptionsTarget for GlobalGameOptionsTarget {
    fn load(&self) -> LaunchConfig {
        self.config.read().launch.clone()
    }

    fn save(&self, update: GameOptionsUpdate) {
        let shell = Rc::clone(&self.shell);
        let page_toast = self.page_toast.clone();
        config_store::store().update(
            move |config| apply_update(&mut config.launch, &update),
            move |err| shell.page_toast(&page_toast, &format!("保存配置失败：{err:#}")),
        );
    }
}
