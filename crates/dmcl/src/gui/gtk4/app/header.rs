//! 标题栏：标签页栏 + 右侧的 账号 / 任务 / 设置 三个 popup。
//!
//! 本期 账号 / 设置 只是占位；任务 popup 还没接任务列表。

use relm4::adw;
use relm4::adw::prelude::*;
use relm4::gtk;

/// 建标题栏。title widget 就是标签页栏（绑定到 `tab_view`）。
pub(crate) fn build(tab_view: &adw::TabView) -> adw::HeaderBar {
    let tab_bar = adw::TabBar::new();
    tab_bar.set_view(Some(tab_view));
    // 只有一个页面时不显示标签栏（也就没有标题）。
    tab_bar.set_autohide(true);

    let bar = adw::HeaderBar::new();
    bar.set_title_widget(Some(&tab_bar));

    let account = menu_button("system-users-symbolic", "账号", "账号功能尚未实现");
    let tasks = menu_button("view-list-symbolic", "任务", "暂无任务");
    let settings = menu_button("preferences-system-symbolic", "设置", "设置功能尚未实现");

    // 从右往左 pack：设置最右、账号最左。
    bar.pack_end(&settings);
    bar.pack_end(&tasks);
    bar.pack_end(&account);
    bar
}

/// 一个图标按钮，点开是一个只有一行说明的 popover（占位）。
fn menu_button(icon: &str, tooltip: &str, text: &str) -> gtk::MenuButton {
    let label = gtk::Label::new(Some(text));
    label.set_margin_top(8);
    label.set_margin_bottom(8);
    label.set_margin_start(12);
    label.set_margin_end(12);

    let popover = gtk::Popover::new();
    popover.set_child(Some(&label));

    let button = gtk::MenuButton::builder()
        .icon_name(icon)
        .tooltip_text(tooltip)
        .build();
    button.set_popover(Some(&popover));
    button
}
