//! 账户分组：从全局账户表里选默认账户。

use relm4::adw;
use relm4::adw::prelude::*;
use relm4::prelude::*;

use crate::config::{AccountConfig, Config, LaunchConfig};
use crate::gui::gtk4::pages::settings::common::{
    account_avatar_text, account_type_label, account_username, ChoiceRow, ChoiceRowInit,
    ChoiceRowMsg, ChoiceRowOut,
};

use super::super::{
    account_for_index, account_index_for, account_label, GameOptionField, GameOptionGroup,
    ACCOUNT_NONE_INDEX,
};
use super::{notify, FixedRow, GameOptionsMsg, GameOptionsPage, GroupView};

pub(super) struct AccountGroup {
    pub(super) expander: adw::ExpanderRow,
    inherit: FixedRow,
    rows: FactoryVecDeque<ChoiceRow>,
    accounts: Vec<(String, AccountConfig)>,
    /// 当前选中的账户下标；账户从 1 开始，0 表示还没有选中账户。
    index: u32,
}

impl AccountGroup {
    pub(super) fn new(allow_inherit: bool, sender: &ComponentSender<GameOptionsPage>) -> Self {
        let expander = adw::ExpanderRow::builder()
            .title("账户")
            .show_enable_switch(false)
            .build();
        let inherit = FixedRow::new("使用全局配置");
        inherit.row.connect_activated({
            let sender = sender.clone();
            move |_| notify(&sender, GameOptionsMsg::Inherit(GameOptionGroup::Account))
        });
        if allow_inherit {
            expander.add_row(&inherit.row);
        }
        let rows = FactoryVecDeque::builder().launch(expander.clone()).forward(
            sender.input_sender(),
            |out| match out {
                ChoiceRowOut::Selected(index) => GameOptionsMsg::AccountSelected(index),
            },
        );
        Self {
            expander,
            inherit,
            rows,
            accounts: Vec::new(),
            index: ACCOUNT_NONE_INDEX,
        }
    }
}

/// 读取全局账户表，按当前配置定位选中项并重建选择行。
pub(super) fn reload(group: &mut AccountGroup, config: &Config, launch: &LaunchConfig) {
    group.accounts = config
        .accounts
        .iter()
        .map(|(name, account)| (name.clone(), account.clone()))
        .collect();
    group.index = account_index_for(&group.accounts, &launch.account);

    let mut guard = group.rows.guard();
    guard.clear();
    for (offset, (_, account)) in group.accounts.iter().enumerate() {
        let index = offset as u32 + 1;
        let username = account_username(account);
        guard.push_back(ChoiceRowInit {
            index,
            title: username.to_string(),
            subtitle: account_type_label(account).to_string(),
            detail: String::new(),
            avatar_text: Some(account_avatar_text(username)),
            selected: false,
        });
    }
}

/// 同步对勾、继承状态与小字。
pub(super) fn sync(group: &AccountGroup, view: GroupView<'_>) {
    for index in 0..group.rows.len() {
        group.rows.send(
            index,
            ChoiceRowMsg::SetSelected(!view.inherited && index as u32 + 1 == group.index),
        );
    }
    group.inherit.set_selected(view.inherited);
    group
        .expander
        .set_subtitle(&view.subtitle(account_label_of(&group.accounts, &view.launch.account)));
    group
        .inherit
        .set_subtitle(&account_label_of(&group.accounts, &view.global.account));
}

/// 选中账户：更新模型、标成覆盖并收起展开行。
pub(super) fn selected(
    page: &mut GameOptionsPage,
    index: u32,
    sender: &ComponentSender<GameOptionsPage>,
) {
    if index == ACCOUNT_NONE_INDEX || index as usize > page.account.accounts.len() {
        return;
    }
    let inherited = page.inherited(GameOptionGroup::Account);
    if index != page.account.index || inherited {
        page.account.index = index;
        page.launch.account = account_for_index(&page.account.accounts, index);
        page.mark_override([GameOptionField::Account]);
        page.schedule_flush(sender);
    }
    page.sync_ui();
    page.account.expander.set_expanded(false);
}

fn account_label_of(accounts: &[(String, AccountConfig)], account: &Option<String>) -> String {
    match account_index_for(accounts, account) {
        ACCOUNT_NONE_INDEX => "未选择".to_string(),
        index => accounts
            .get((index - 1) as usize)
            .map(|(name, account)| account_label(name, account))
            .unwrap_or_else(|| "未选择".to_string()),
    }
}
