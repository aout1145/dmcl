//! 分类选择器：展开行 + 已有分类 + 自定义输入。
//!
//! 新建实例页与实例设置页共用。用户选已有分类、取消选择时发 [`CategoryPickerOut::Selected`]；
//! 在自定义输入框里打字时发 [`CategoryPickerOut::Edited`]，让需要防抖保存的页面自己安排写入。

use relm4::adw;
use relm4::adw::prelude::*;
use relm4::gtk;
use relm4::prelude::*;

const UNCATEGORIZED: &str = "未分类";

/// 组件初始化参数。
pub(crate) struct CategoryPickerInit {
    pub(crate) categories: Vec<String>,
    pub(crate) category: String,
}

/// 组件输入。
#[derive(Debug)]
pub(crate) enum CategoryPickerMsg {
    /// 自定义输入框内容变化。
    Edited(String),
    /// 选中已有分类。
    Select(String),
    /// 取消分类。
    Clear,
    /// 程序设置分类（不产生输出）。
    SetCategory(String),
}

/// 组件输出。
#[derive(Debug)]
pub(crate) enum CategoryPickerOut {
    /// 通过选择已有分类或取消选择确定的分类。
    Selected(String),
    /// 输入框里的分类正在编辑。
    Edited(String),
}

pub(crate) struct CategoryPicker {
    expander: adw::ExpanderRow,
    entry: adw::EntryRow,
    clear: gtk::Button,
    category: String,
    /// 正在程序写输入框；忽略随之而来的 `changed`。
    programmatic: bool,
    choices: FactoryVecDeque<CategoryChoice>,
}

/// 展开区里的一条已有分类。
struct CategoryChoice {
    category: String,
    selected: bool,
}

#[relm4::factory]
impl FactoryComponent for CategoryChoice {
    type Init = (String, bool);
    type Input = bool;
    type Output = String;
    type CommandOutput = ();
    type ParentWidget = adw::ExpanderRow;
    type Index = DynamicIndex;

    view! {
        adw::ActionRow {
            add_css_class: "dmcl-category-choice",
            #[watch]
            set_title: &self.category,
            set_activatable: true,

            add_suffix = &gtk::Image {
                set_icon_name: Some("object-select-symbolic"),
                #[watch]
                set_visible: self.selected,
                add_css_class: "accent",
                set_valign: gtk::Align::Center,
            },

            connect_activated[sender] => move |_| {
                let _ = sender.output(category.clone());
            },
        }
    }

    fn init_model(
        (category, selected): Self::Init,
        _index: &DynamicIndex,
        _sender: FactorySender<Self>,
    ) -> Self {
        Self { category, selected }
    }

    fn init_widgets(
        &mut self,
        _index: &DynamicIndex,
        _root: Self::Root,
        _returned_widget: &gtk::Widget,
        sender: FactorySender<Self>,
    ) -> Self::Widgets {
        let category = self.category.clone();
        let widgets = view_output!();
        widgets
    }

    fn update(&mut self, selected: Self::Input, _sender: FactorySender<Self>) {
        self.selected = selected;
    }
}

impl CategoryPicker {
    fn subtitle(&self) -> &str {
        if self.category.trim().is_empty() {
            UNCATEGORIZED
        } else {
            &self.category
        }
    }

    /// 分类文字变化后同步收起状态的副标题、取消按钮和对勾。
    fn sync_view(&mut self) {
        let selected = self.category.trim();
        self.expander.set_subtitle(if selected.is_empty() {
            UNCATEGORIZED
        } else {
            selected
        });
        self.clear.set_visible(!selected.is_empty());

        let mut guard = self.choices.guard();
        for choice in guard.iter_mut() {
            choice.selected = choice.category.as_str() == selected;
        }
    }

    /// 写回输入框并同步选择标记；写之前先改模型，避免 `changed` 被当成一次新编辑。
    fn apply_category(&mut self, category: String) {
        self.category = category;
        if self.entry.text() != self.category {
            self.programmatic = true;
            self.entry.set_text(&self.category);
            self.programmatic = false;
        }
        self.sync_view();
    }
}

#[relm4::component(pub(crate))]
impl SimpleComponent for CategoryPicker {
    type Init = CategoryPickerInit;
    type Input = CategoryPickerMsg;
    type Output = CategoryPickerOut;

    view! {
        adw::ExpanderRow {
            set_title: "分类",
            #[watch]
            set_subtitle: model.subtitle(),
            set_show_enable_switch: false,
        }
    }

    fn init(
        init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let CategoryPickerInit {
            categories,
            category,
        } = init;

        let clear = gtk::Button::from_icon_name("window-close-symbolic");
        clear.add_css_class("flat");
        clear.set_valign(gtk::Align::Center);
        clear.set_tooltip_text(Some("取消选择（未分类）"));
        clear.set_visible(false);
        clear.connect_clicked({
            let sender = sender.clone();
            move |_| sender.input(CategoryPickerMsg::Clear)
        });
        root.add_suffix(&clear);

        let entry = adw::EntryRow::builder().title("自定义分类").build();
        entry.set_tooltip_text(Some("留空表示未分类"));
        entry.set_text(&category);
        entry.connect_changed({
            let sender = sender.clone();
            move |row| sender.input(CategoryPickerMsg::Edited(row.text().into()))
        });

        let mut choices = FactoryVecDeque::builder()
            .launch(root.clone())
            .forward(sender.input_sender(), CategoryPickerMsg::Select);
        {
            let mut guard = choices.guard();
            for category in categories {
                guard.push_back((category, false));
            }
        }
        // 自定义输入放最下面：已有分类先排，最后才是自由输入。
        root.add_row(&entry);

        let mut model = Self {
            expander: root.clone(),
            entry,
            clear,
            category,
            programmatic: false,
            choices,
        };
        model.sync_view();

        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>) {
        match message {
            CategoryPickerMsg::Edited(text) => {
                if self.programmatic || text == self.category {
                    return;
                }
                self.category = text.clone();
                self.sync_view();
                let _ = sender.output(CategoryPickerOut::Edited(text));
            }
            CategoryPickerMsg::Select(category) => {
                self.apply_category(category.clone());
                self.expander.set_expanded(false);
                let _ = sender.output(CategoryPickerOut::Selected(category));
            }
            CategoryPickerMsg::Clear => {
                self.apply_category(String::new());
                let _ = sender.output(CategoryPickerOut::Selected(String::new()));
            }
            CategoryPickerMsg::SetCategory(category) => self.apply_category(category),
        }
    }
}
