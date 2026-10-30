//! 新建实例页的消息。

use crate::gui::gtk4::pages::download::common::version_select::VersionItem;

/// 页面消息。
#[derive(Debug)]
pub(crate) enum CustomMsg {
    /// Minecraft 版本下拉的选择变化。
    McChanged(Option<VersionItem>),
    /// 第 `slot` 个加载器下拉的选择变化。
    LoaderChanged {
        slot: usize,
        item: Option<VersionItem>,
    },
    /// 实例名称显示值变化（自动命名 / 用户编辑都会发）。
    NameChanged(String),
    /// 用户点了安装按钮。
    Install { name: String, category: String },
}
