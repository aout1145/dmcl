//! 主页：已安装实例的网格。

pub(crate) mod actions_popup;
pub(crate) mod instance_card;

use relm4::gtk;
use relm4::gtk::prelude::*;
use relm4::prelude::*;

use crate::gui::gtk4::app::AppModel;

/// 一个实例在界面上的视图。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InstanceView {
    pub name: String,
    pub running: bool,
}

/// 建主页内容：一个可滚动的实例网格。返回（可滚动容器，网格本身）。
pub(crate) fn build() -> (gtk::ScrolledWindow, gtk::FlowBox) {
    let grid = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .valign(gtk::Align::Start)
        .halign(gtk::Align::Start)
        .row_spacing(12)
        .column_spacing(12)
        .margin_top(18)
        .margin_bottom(18)
        .margin_start(18)
        .margin_end(18)
        .build();

    let scrolled = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&grid)
        .vexpand(true)
        .build();

    (scrolled, grid)
}

/// 按模型重建网格内容。
pub(crate) fn render(
    grid: &gtk::FlowBox,
    instances: &[InstanceView],
    sender: &ComponentSender<AppModel>,
) {
    while let Some(child) = grid.first_child() {
        grid.remove(&child);
    }

    if instances.is_empty() {
        let empty = gtk::Label::new(Some("还没有安装任何版本。"));
        empty.add_css_class("dim-label");
        empty.set_margin_top(24);
        grid.append(&empty);
        return;
    }

    for instance in instances {
        grid.append(&instance_card::build(instance, sender));
    }
}
