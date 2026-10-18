//! 标题栏：一行里排「添加实例」+ 标签页栏 + 账号 / 任务 / 设置 + 窗口按钮。
//!
//! 整行都放在 `AdwHeaderBar` 的 title widget 里自己排，原因见 [`build`]：头栏把 title widget
//! 放在 `GtkCenterBox` 中间、左右各让出「较宽那一侧」的宽度，若只把标签栏当 title widget，
//! 左边会被右侧的按钮挤出一大截空白。
//!
//! 账号 / 设置 是占位 popup；任务按钮由 root 接到「任务对话框」上；「添加实例」的下拉由
//! root 接到页面宿主（见 [`Header::add_custom`]）。

use relm4::adw;
use relm4::adw::prelude::*;
use relm4::gtk;

/// 标题栏的样式补丁（全局 CSS）：
/// * 收紧标签栏竖向内边距，让标题栏保持 libadwaita 正常的 47px 高度；
/// * 隐藏**主页**标签的关闭按钮——libadwaita 没有 per-page 的 closable，也没有公开句柄
///   拿到某个标签的关闭按钮，只能按 CSS 节点把 class 打在主页那个标签上（见
///   [`mark_home_tab`]），其余标签保持可关闭。
pub(crate) const CSS: &str = "\
headerbar.dmcl-header tabbar tabbox { padding-top: 0; padding-bottom: 0; }
headerbar.dmcl-header tabbar .box { padding: 0; }
headerbar.dmcl-header tabbar tab.dmcl-home-tab button.image-button { min-width: 0; min-height: 0; padding: 0; margin: 0; -gtk-icon-size: 1px; opacity: 0; }";

/// 标题栏及其需要被后续引用的部件。
pub(crate) struct Header {
    pub bar: adw::HeaderBar,
    /// 标签页栏：root 用来给主页标签打 class。
    pub tab_bar: adw::TabBar,
    /// 任务按钮：点开任务对话框。
    pub tasks_button: gtk::Button,
    /// 「添加实例」下拉里的「自定义」项：点开一个「新实例」页。
    pub add_custom: gtk::Button,
}

/// 建标题栏。
///
/// 整行（加号 / 标签栏 / 右侧按钮 / 窗口按钮）都放进 title widget 里自己排，而不是分头
/// `pack_start` / `pack_end`。因为 `AdwHeaderBar` 把 title widget 摆在 `GtkCenterBox` 正中：
/// 中间宽度 = 总宽 − 2×max(左, 右)。左侧只有一个加号（40px），右侧是三个按钮加窗口按钮
/// （实测 234px），于是标签栏右边贴着按钮、左边空出近 200px。把两侧内容也并进 title widget、
/// 并关掉头栏自带的 start/end 标题按钮之后，左右两侧宽度都是 0，标签栏就能从加号一直铺到
/// 窗口按钮。
pub(crate) fn build(tab_view: &adw::TabView) -> Header {
    let tab_bar = adw::TabBar::new();
    tab_bar.set_view(Some(tab_view));
    // 单个标签时也一直显示标签栏：否则从 2 个标签关到 1 个时，整条标签栏会跟着
    // `GtkRevealer`（slide-down，200ms）滑走，和标签自身的收缩动画叠在一起，看着不顺。
    tab_bar.set_autohide(false);

    let bar = adw::HeaderBar::new();
    bar.add_css_class("dmcl-header");
    bar.set_show_start_title_buttons(false);
    bar.set_show_end_title_buttons(false);

    // 「添加实例」：最左的 + 按钮，点开是下载类型列表（目前只有「自定义」）。
    let add_custom = gtk::Button::with_label("自定义");
    add_custom.add_css_class("flat");
    add_custom.set_margin_top(6);
    add_custom.set_margin_bottom(6);
    add_custom.set_margin_start(6);
    add_custom.set_margin_end(6);
    let add_list = gtk::Box::new(gtk::Orientation::Vertical, 0);
    add_list.append(&add_custom);
    let add_popover = gtk::Popover::new();
    add_popover.set_child(Some(&add_list));
    // 默认是「以按钮为中心」展开；加号在最左边，居中的话菜单会一半探到窗口外再被挤回来。
    // 改成左对齐：菜单左边缘贴住加号（也就是窗口左边框，只差标题栏自带的 7px 内边距）。
    add_popover.set_halign(gtk::Align::Start);
    let add_button = gtk::MenuButton::builder()
        .icon_name("list-add-symbolic")
        .tooltip_text("添加实例")
        .popover(&add_popover)
        .build();
    // 自定义按钮不是 model button，popover 不会自己收起来。
    {
        let add_popover = add_popover.clone();
        add_custom.connect_clicked(move |_| add_popover.popdown());
    }

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

    // 窗口按钮：头栏自带的那套关掉了（见函数头），这里自己加在行尾。
    // `decoration-layout` 默认跟随桌面设置；桌面不画窗口按钮时它自己宽度为 0。
    let controls = gtk::WindowControls::new(gtk::PackType::End);

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.append(&add_button);
    row.append(&tab_bar);
    row.append(&account);
    row.append(&tasks_button);
    row.append(&settings);
    row.append(&controls);
    bar.set_title_widget(Some(&row));

    Header {
        bar,
        tab_bar,
        tasks_button,
        add_custom,
    }
}

/// 给主页的标签节点打上 `.dmcl-home-tab`，好让 [`CSS`] 只隐藏它的关闭按钮。
///
/// 标签栏的 CSS 节点链是 `tabbar` → `tab` → `button.image-button`。主页是第一个标签，
/// 所以找第一个名为 `tab` 的后代即可（**要在标签建好之后调用**）。
pub(crate) fn mark_home_tab(tab_bar: &adw::TabBar) {
    if let Some(tab) = find_css_node(tab_bar.upcast_ref(), "tab") {
        tab.add_css_class("dmcl-home-tab");
    }
}

/// 深度优先找第一个 CSS 名为 `name` 的后代。
fn find_css_node(widget: &gtk::Widget, name: &str) -> Option<gtk::Widget> {
    let mut child = widget.first_child();
    while let Some(current) = child {
        if current.css_name().as_str() == name {
            return Some(current);
        }
        if let Some(found) = find_css_node(&current, name) {
            return Some(found);
        }
        child = current.next_sibling();
    }
    None
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
