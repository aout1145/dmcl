//! 设置页：首选项与单个实例的实例设置。
//!
//! 两者都是 GNOME 设置式的左右分栏，左侧分类列表、右侧分类内容；具体分类与内容在
//! [`launcher`]、[`game`] 和 [`instance`] 里，共用交互在 [`common`] 里。首选项由根组件在
//! 启动时创建并通过 `PageSlot` 常驻：关闭标签只影响显示，重新打开仍是同一个页面；实例设置
//! 页是一次性页面，按 `(root, 实例名)` 去重，关闭即销毁。

mod common;
mod game;
mod instance;
mod launcher;

pub(crate) use common::{JavaProbeCache, CSS};
pub(crate) use game::{
    copy_fields, GameOptionField, GameOptionsInit, GameOptionsMsg, GameOptionsPage,
    GameOptionsState, GameOptionsTarget, GameOptionsUpdate,
};
pub(crate) use instance::{InstanceMsg, InstancePage};
pub(crate) use launcher::{AccountPage, DirectoryPage, PreferencesMsg, PreferencesPage};
