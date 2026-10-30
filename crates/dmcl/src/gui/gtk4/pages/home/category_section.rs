//! 主页里按分类分组的一组实例磁贴。
//!
//! 每个分类是一段「标题 + 自动换行磁贴」，磁贴本身仍然是 [`InstanceCard`]。分组只改变
//! 展示层级，选中 / 启动仍然按实例名交给主页处理。

use relm4::adw;
use relm4::gtk;
use relm4::gtk::prelude::*;
use relm4::prelude::*;

use super::instance_card::{CardOut, InstanceCard};
use super::InstanceState;

/// 没有分类（或分类为空白）的实例使用的组名。
pub(crate) const UNCATEGORIZED: &str = "未分类";

/// 一个分类分组。
pub(crate) struct CategorySection {
    title: String,
    pub(crate) cards: FactoryVecDeque<InstanceCard>,
}

#[relm4::factory(pub(crate))]
impl FactoryComponent for CategorySection {
    type Init = (String, Vec<(String, InstanceState, bool)>);
    type Input = ();
    type Output = CardOut;
    type CommandOutput = ();
    type ParentWidget = gtk::Box;
    type Index = DynamicIndex;

    view! {
        root = gtk::Box {
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 8,

            gtk::Label {
                set_xalign: 0.0,
                set_margin_start: 6,
                add_css_class: "heading",
                set_label: &self.title,
            },

            #[local_ref]
            cards_widget -> adw::WrapBox {
                set_child_spacing: 12,
                set_line_spacing: 12,
            },
        }
    }

    fn init_model(
        (title, entries): Self::Init,
        _index: &DynamicIndex,
        sender: FactorySender<Self>,
    ) -> Self {
        let mut cards = FactoryVecDeque::builder()
            .launch(adw::WrapBox::new())
            .forward(sender.output_sender(), |out| out);
        {
            let mut guard = cards.guard();
            for entry in entries {
                guard.push_back(entry);
            }
        }
        Self { title, cards }
    }

    fn init_widgets(
        &mut self,
        _index: &DynamicIndex,
        _root: Self::Root,
        _returned_widget: &gtk::Widget,
        _sender: FactorySender<Self>,
    ) -> Self::Widgets {
        let cards_widget = self.cards.widget();
        let widgets = view_output!();
        widgets
    }
}

/// 按分类把实例分组。
///
/// 分组顺序按分类第一次出现的顺序；没有分类的实例全部放进最后的「未分类」。
/// 空分组不会被创建。
pub(crate) fn group_instances(
    instances: Vec<(String, Option<String>)>,
) -> Vec<(String, Vec<String>)> {
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    let mut uncategorized = Vec::new();

    for (name, category) in instances {
        match category
            .map(|category| category.trim().to_string())
            .filter(|category| !category.is_empty())
        {
            Some(category) => match groups.iter_mut().find(|(title, _)| *title == category) {
                Some((_, names)) => names.push(name),
                None => groups.push((category, vec![name])),
            },
            None => uncategorized.push(name),
        }
    }
    if !uncategorized.is_empty() {
        groups.push((UNCATEGORIZED.to_string(), uncategorized));
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::group_instances;

    #[test]
    fn groups_follow_first_appearance_and_uncategorized_goes_last() {
        let grouped = group_instances(vec![
            ("a".to_string(), Some("整合包".to_string())),
            ("b".to_string(), None),
            ("c".to_string(), Some(" 冒险 ".to_string())),
            ("d".to_string(), Some("整合包".to_string())),
            ("e".to_string(), Some("   ".to_string())),
        ]);
        assert_eq!(
            grouped,
            vec![
                ("整合包".to_string(), vec!["a".to_string(), "d".to_string()]),
                ("冒险".to_string(), vec!["c".to_string()]),
                ("未分类".to_string(), vec!["b".to_string(), "e".to_string()]),
            ]
        );
    }

    #[test]
    fn empty_input_and_only_categories_have_no_empty_groups() {
        assert!(group_instances(Vec::new()).is_empty());
        assert_eq!(
            group_instances(vec![("a".to_string(), Some("原版".to_string()))]),
            vec![("原版".to_string(), vec!["a".to_string()])]
        );
    }
}
