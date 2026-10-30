//! 单个实例磁贴（工厂组件）：头像 + 名称；运行中时右上角有小绿点。
//!
//! 磁贴只负责渲染自己的模型，选中/启动由父组件决定；点击和双击通过
//! [`CardOut`] 上报。

use relm4::adw;
use relm4::gtk;
use relm4::gtk::prelude::*;
use relm4::prelude::*;

use super::InstanceState;

/// 磁贴发给主页的消息。直接带实例名，分组重建后索引变化也不会指错。
#[derive(Debug)]
pub(crate) enum CardOut {
    Select(String),
    Launch(String),
}

/// 一个实例磁贴的模型。
#[derive(Debug)]
pub(crate) struct InstanceCard {
    pub(crate) name: String,
    pub(crate) state: InstanceState,
    pub(crate) selected: bool,
}

impl InstanceCard {
    fn css_classes(&self) -> Vec<&'static str> {
        let mut classes = vec!["flat"];
        if self.selected {
            classes.push("dmcl-instance-selected");
        }
        classes
    }
}

#[relm4::factory(pub(crate))]
impl FactoryComponent for InstanceCard {
    type Init = (String, InstanceState, bool);
    type Input = ();
    type Output = CardOut;
    type CommandOutput = ();
    type ParentWidget = adw::WrapBox;
    type Index = DynamicIndex;

    view! {
        root = gtk::Button {
            set_has_frame: false,
            set_hexpand: false,
            set_width_request: 136,
            set_tooltip_text: Some(&self.name),
            #[watch]
            set_css_classes: &self.css_classes(),

            gtk::Overlay {
                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 4,
                    set_halign: gtk::Align::Center,
                    set_margin_top: 6,
                    set_margin_bottom: 6,
                    set_margin_start: 8,
                    set_margin_end: 8,

                    // 首字母彩色头像（暂无实例图标来源）。
                    adw::Avatar {
                        set_size: 48,
                        set_text: Some(&self.name),
                        set_show_initials: true,
                    },

                    // 最多三行，和 GNOME Files 一样；短名字不会硬撑高度。
                    gtk::Label {
                        set_label: &self.name,
                        set_justify: gtk::Justification::Center,
                        set_wrap: true,
                        set_wrap_mode: gtk::pango::WrapMode::WordChar,
                        set_lines: 3,
                        set_ellipsize: gtk::pango::EllipsizeMode::End,
                        set_width_chars: 12,
                        set_max_width_chars: 12,
                        set_yalign: 0.0,
                    },
                },

                // 运行中：右上角的小绿点（symbolic 圆点 + success 颜色）。
                add_overlay = &gtk::Image {
                    set_icon_name: Some("media-record-symbolic"),
                    add_css_class: "success",
                    set_pixel_size: 20,
                    set_halign: gtk::Align::End,
                    set_valign: gtk::Align::Start,
                    set_margin_top: 6,
                    set_margin_end: 4,
                    // 绿点只表示游戏进程真的在跑，启动链阶段不算。
                    #[watch]
                    set_visible: self.state == InstanceState::Running,
                },
            },
        }
    }

    fn init_model(
        (name, state, selected): Self::Init,
        _index: &DynamicIndex,
        _sender: FactorySender<Self>,
    ) -> Self {
        Self {
            name,
            state,
            selected,
        }
    }

    fn init_widgets(
        &mut self,
        _index: &DynamicIndex,
        root: Self::Root,
        _returned_widget: &gtk::Widget,
        sender: FactorySender<Self>,
    ) -> Self::Widgets {
        let button = root.clone();
        let widgets = view_output!();
        let name = self.name.clone();

        button.connect_clicked({
            let sender = sender.clone();
            let name = name.clone();
            move |_| {
                let _ = sender.output(CardOut::Select(name.clone()));
            }
        });

        // 双击启动：捕获阶段处理，避免被按钮自己的 clicked 吞掉。
        let double_click = gtk::GestureClick::new();
        double_click.set_propagation_phase(gtk::PropagationPhase::Capture);
        double_click.connect_pressed({
            let sender = sender.clone();
            move |_, n_press, _, _| {
                if n_press >= 2 {
                    let _ = sender.output(CardOut::Launch(name.clone()));
                }
            }
        });
        button.add_controller(double_click);

        widgets
    }
}
