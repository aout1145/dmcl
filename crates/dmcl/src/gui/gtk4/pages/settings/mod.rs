//! 设置页：首选项。
//!
//! 页面是 GNOME 设置式的左右分栏，左侧分类列表、右侧分类内容；具体分类与内容在
//! [`launcher`] 和 [`game`] 里，共用交互在 [`common`] 里。首选项页由根组件在启动时创建，
//! 并通过 `PageSlot` 常驻：关闭标签只影响显示，重新打开仍是同一个页面。

mod common;
mod game;
mod launcher;

pub(crate) use common::CSS;
pub(crate) use launcher::{PreferencesMsg, PreferencesPage};
