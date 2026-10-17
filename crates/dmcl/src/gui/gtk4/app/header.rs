//! 标题栏：标签页栏 + 右侧的 账号 / 任务 / 设置 三个按钮。
//!
//! 账号 / 设置 是占位 popup；任务按钮由 root 接到「任务对话框」上。

use relm4::adw;
use relm4::adw::prelude::*;
use relm4::gtk;

/// 标题栏的样式补丁（全局 CSS）：
/// * 收紧标签栏竖向内边距，让标题栏保持 libadwaita 正常的 47px 高度；
/// * 隐藏标签的关闭按钮——libadwaita 没有 per-page 的 closable，唯一没有关闭按钮的方式是
///   `pinned`，但 `pinned` 又不显示标题；当前不支持关闭标签，直接隐藏 `×`。
pub(crate) const CSS: &str = "\
headerbar.dmcl-header tabbar tabbox { padding-top: 0; padding-bottom: 0; }
headerbar.dmcl-header tabbar .box { padding: 0; }
headerbar.dmcl-header tabbar tab button.image-button { min-width: 0; min-height: 0; padding: 0; margin: 0; -gtk-icon-size: 1px; opacity: 0; }";

/// 标题栏及其需要被后续引用的部件。
pub(crate) struct Header {
    pub bar: adw::HeaderBar,
    /// 任务按钮：点开任务对话框。
    pub tasks_button: gtk::Button,
}

/// 建标题栏。title widget 就是标签页栏（绑定到 `tab_view`）。
pub(crate) fn build(tab_view: &adw::TabView) -> Header {
    let tab_bar = adw::TabBar::new();
    tab_bar.set_view(Some(tab_view));
    // 只有一个页面时不显示标签栏（也就没有标题）。
    tab_bar.set_autohide(true);

    let bar = adw::HeaderBar::new();
    bar.add_css_class("dmcl-header");
    bar.set_title_widget(Some(&tab_bar));

    let account = menu_button(
        "system-users-symbolic",
        "账号",
        &placeholder("账号功能尚未实现"),
    );

    let tasks_button = gtk::Button::builder()
        .icon_name("view-list-symbolic")
        .tooltip_text("任务")
        .build();
    tasks_button.add_css_class("flat");

    let settings = menu_button(
        "preferences-system-symbolic",
        "设置",
        &placeholder("设置功能尚未实现"),
    );

    // 从右往左 pack：设置最右、账号最左。
    bar.pack_end(&settings);
    bar.pack_end(&tasks_button);
    bar.pack_end(&account);

    Header { bar, tasks_button }
}

/// 一个图标按钮，点开是一个装着 `content` 的 popover。
fn menu_button(icon: &str, tooltip: &str, content: &impl IsA<gtk::Widget>) -> gtk::MenuButton {
    let popover = gtk::Popover::new();
    popover.set_child(Some(content));

    let button = gtk::MenuButton::builder()
        .icon_name(icon)
        .tooltip_text(tooltip)
        .build();
    button.set_popover(Some(&popover));
    button
}

/// 占位 popover 里的一行说明。
fn placeholder(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_margin_top(8);
    label.set_margin_bottom(8);
    label.set_margin_start(12);
    label.set_margin_end(12);
    label
}
