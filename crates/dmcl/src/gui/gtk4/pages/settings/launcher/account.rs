//! 首选项里的「账号」分类：账户列表、添加离线账户、微软登录与删除。

use std::rc::Rc;

use dmcl_launch::microsoft::{
    device_code_login, DeviceCodeLogin, DeviceCodePresenter, DeviceCodePrompt,
};
use dmcl_launch::{RefreshTokenStore, StoredCredential};
use relm4::adw;
use relm4::adw::prelude::*;
use relm4::gtk;
use relm4::prelude::*;

use crate::config::{unique_account_name, AccountConfig, Config, KeyringTokenStore};
use crate::gui::gtk4::app::{config_store, ConfigRef, PageToast, Shell};
use crate::gui::gtk4::pages::settings::common::{
    account_avatar_text, account_type_label, account_username,
};

/// 「账号」分类收到的消息。
#[derive(Debug)]
pub(crate) enum AccountMsg {
    /// 共享配置变了：重新读取账户表。
    ConfigChanged,
    /// 打开添加离线账户的对话框。
    AddOffline,
    /// 添加离线账户的对话框确认了玩家名。
    SaveOffline(String),
    /// 开始 Microsoft 设备代码登录。
    LoginMicrosoft,
    /// 设备代码就绪，展示给用户。
    MicrosoftPrompt {
        generation: u64,
        prompt: DeviceCodePrompt,
    },
    /// 设备代码登录结束。
    MicrosoftDone {
        generation: u64,
        result: Result<DeviceCodeLogin, String>,
    },
    /// Microsoft 登录提示被用户关掉了。
    LoginDialogClosed,
    /// 微软凭据已经写入钥匙串（或写入失败），可以更新配置了。
    MicrosoftCredentialSaved {
        key: String,
        username: String,
        uuid: String,
        error: Option<String>,
    },
    /// 把某个账户设为默认（写入 `launch.account`）。
    SetDefault(String),
    /// 请求删除某个账户（先弹确认框）。
    Remove(String),
    /// 确认框里点了删除。
    RemoveConfirmed(String),
    /// 账户已经删掉，但清理钥匙串凭据失败。
    KeyringCleanupFailed(String),
}

pub(crate) struct AccountPage {
    config: ConfigRef,
    shell: Rc<Shell>,
    /// 页面级提示：只有首选项可见时才显示。
    page_toast: PageToast,
    /// 每个账户一行；由配置顺序决定先后。
    accounts: FactoryVecDeque<AccountRow>,
    /// 固定在列表最后的「添加账户」行。
    actions_row: adw::ActionRow,
    /// 微软登录按钮；登录进行时置灰。
    login_button: gtk::Button,
    /// 正在展示的 Microsoft 登录提示。
    login_dialog: Option<adw::Dialog>,
    /// 登录提示里的内容容器；拿到设备代码后换成代码说明。
    login_content: Option<adw::ToolbarView>,
    /// 后台登录任务；关掉提示框时中止。
    login_task: Option<tokio::task::JoinHandle<()>>,
    /// 已经发起登录，避免重复点击。
    logging_in: bool,
    /// 每次登录 / 关闭提示框都会递增，用来丢弃过期的提示和结果。
    login_generation: u64,
}

#[relm4::component(pub(crate))]
impl SimpleComponent for AccountPage {
    type Init = Rc<Shell>;
    type Input = AccountMsg;
    type Output = ();

    view! {
        adw::PreferencesPage {
            #[local_ref]
            accounts_group -> adw::PreferencesGroup {},
        }
    }

    fn init(
        shell: Self::Init,
        root_widget: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let config = config_store::store().config();
        let group = adw::PreferencesGroup::builder()
            .title("账户")
            .description("管理启动器可用的账户。")
            .build();

        let accounts = FactoryVecDeque::builder().launch(group.clone()).forward(
            sender.input_sender(),
            |out| match out {
                AccountRowOut::SetDefault(key) => AccountMsg::SetDefault(key),
                AccountRowOut::Remove(key) => AccountMsg::Remove(key),
            },
        );

        let add_button = gtk::Button::with_label("添加离线账户");
        add_button.add_css_class("flat");
        add_button.connect_clicked({
            let sender = sender.clone();
            move |_| sender.input(AccountMsg::AddOffline)
        });

        let login_button = gtk::Button::with_label("登录微软账户");
        login_button.add_css_class("flat");
        login_button.connect_clicked({
            let sender = sender.clone();
            move |_| sender.input(AccountMsg::LoginMicrosoft)
        });

        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        actions.set_halign(gtk::Align::End);
        actions.set_valign(gtk::Align::Center);
        actions.append(&add_button);
        actions.append(&login_button);

        let actions_row = adw::ActionRow::builder().title("添加账户").build();
        actions_row.set_activatable(false);
        actions_row.add_suffix(&actions);
        group.add(&actions_row);

        let mut model = AccountPage {
            config,
            shell,
            page_toast: PageToast::new(&root_widget),
            accounts,
            actions_row,
            login_button,
            login_dialog: None,
            login_content: None,
            login_task: None,
            logging_in: false,
            login_generation: 0,
        };
        let accounts_group = model.accounts.widget();
        let widgets = view_output!();
        model.reload();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>) {
        match message {
            AccountMsg::ConfigChanged => self.reload(),
            AccountMsg::AddOffline => show_add_offline(&sender),
            AccountMsg::SaveOffline(username) => {
                let name = {
                    let config = self.config.read();
                    unique_account_name(&config.accounts, &username)
                };
                self.update_config(move |config| {
                    config.add_account(&name, AccountConfig::Offline { username })
                });
                self.reload();
            }
            AccountMsg::LoginMicrosoft => self.start_microsoft_login(&sender),
            AccountMsg::MicrosoftPrompt { generation, prompt } => {
                if generation == self.login_generation {
                    self.show_login_prompt(&prompt);
                }
            }
            AccountMsg::MicrosoftDone { generation, result } => {
                if generation != self.login_generation {
                    return;
                }
                self.login_task = None;
                self.close_login_prompt();
                self.logging_in = false;
                self.login_button.set_sensitive(true);
                match result {
                    Ok(login) => self.save_microsoft_login(login, &sender),
                    Err(err) => self.toast(&format!("Microsoft 登录失败：{err}")),
                }
            }
            AccountMsg::LoginDialogClosed => {
                if self.login_dialog.take().is_some() {
                    self.cancel_login();
                }
            }
            AccountMsg::MicrosoftCredentialSaved {
                key,
                username,
                uuid,
                error,
            } => {
                if let Some(error) = error {
                    self.toast(&format!("保存 Microsoft 登录凭据失败：{error}"));
                    return;
                }
                let name = username.clone();
                self.update_config(move |config| {
                    config.add_account(&key, AccountConfig::Microsoft { username, uuid })
                });
                self.reload();
                self.toast(&format!("已添加 Microsoft 账户：{name}"));
            }
            AccountMsg::Remove(key) => {
                let account = self.config.read().accounts.get(&key).cloned();
                let Some(account) = account else {
                    return;
                };
                confirm_remove(&key, account_username(&account), &sender);
            }
            AccountMsg::SetDefault(key) => {
                {
                    let config = self.config.read();
                    if config.launch.account.as_deref() == Some(key.as_str()) {
                        return;
                    }
                }
                self.update_config(|config| {
                    config.launch.account = Some(key);
                    Ok(())
                });
                self.reload();
            }
            AccountMsg::RemoveConfirmed(key) => {
                let account = self.config.read().accounts.get(&key).cloned();
                let Some(account) = account else {
                    return;
                };
                self.update_config(|config| config.remove_account(&key));
                self.reload();
                if matches!(account, AccountConfig::Microsoft { .. }) {
                    let key = key.clone();
                    let sender = sender.clone();
                    relm4::spawn(async move {
                        if let Err(err) = KeyringTokenStore::new(key).delete().await {
                            sender.input(AccountMsg::KeyringCleanupFailed(format!("{err:#}")));
                        }
                    });
                }
            }
            AccountMsg::KeyringCleanupFailed(err) => {
                self.toast(&format!("已删除账户，但清理系统钥匙串凭据失败：{err}"));
            }
        }
    }
}

impl AccountPage {
    /// 按共享配置重建行；操作按钮固定在最后。
    fn reload(&mut self) {
        let (accounts, default): (Vec<(String, AccountConfig)>, Option<String>) = {
            let config = self.config.read();
            let accounts = config
                .accounts
                .iter()
                .map(|(name, account)| (name.clone(), account.clone()))
                .collect();
            (accounts, config.launch.account.clone())
        };

        let group = self.accounts.widget().clone();
        {
            let mut guard = self.accounts.guard();
            guard.clear();
            group.remove(&self.actions_row);
            for (name, account) in accounts {
                let is_default = default.as_deref() == Some(name.as_str());
                guard.push_back((name, account, is_default));
            }
        }
        group.add(&self.actions_row);
    }

    /// 改配置并异步落盘；失败时在页面上提示。
    fn update_config(&self, f: impl FnOnce(&mut Config) -> anyhow::Result<()>) {
        let mut error = None;
        let shell = self.shell.clone();
        let page_toast = self.page_toast.clone();
        config_store::store().update(
            |config| {
                if let Err(err) = f(config) {
                    error = Some(err);
                }
            },
            move |err| shell.page_toast(&page_toast, &format!("保存配置失败：{err:#}")),
        );
        if let Some(err) = error {
            self.toast(&format!("无法修改账户：{err:#}"));
        }
    }

    fn toast(&self, text: &str) {
        self.shell.page_toast(&self.page_toast, text);
    }

    /// 先弹出登录窗口，再后台请求设备代码；提示代码和结果都回到本组件。
    fn start_microsoft_login(&mut self, sender: &ComponentSender<Self>) {
        if self.logging_in {
            return;
        }
        self.logging_in = true;
        self.login_generation = self.login_generation.wrapping_add(1);
        let generation = self.login_generation;
        self.login_button.set_sensitive(false);

        self.open_login_dialog(sender);

        let presenter = GuiPresenter {
            sender: sender.clone(),
            generation,
        };
        let sender = sender.clone();
        let task = relm4::spawn(async move {
            let result = async {
                KeyringTokenStore::ensure_available().await?;
                device_code_login(&presenter).await
            }
            .await;
            sender.input(AccountMsg::MicrosoftDone {
                generation,
                result: result.map_err(|err| format!("{err:#}")),
            });
        });
        self.login_task = Some(task);
    }

    /// 登录成功后先把凭据写进钥匙串，确认成功再写配置，避免留下指向空凭据的账户。
    fn save_microsoft_login(&self, login: DeviceCodeLogin, sender: &ComponentSender<Self>) {
        let key = {
            let config = self.config.read();
            unique_account_name(&config.accounts, &login.account.username)
        };
        let username = login.account.username.clone();
        let uuid = login.account.uuid.clone();
        let credential = StoredCredential {
            uuid: uuid.clone(),
            refresh_token: login.refresh_token,
        };

        let sender = sender.clone();
        relm4::spawn(async move {
            let error = KeyringTokenStore::new(&key)
                .save(&credential)
                .await
                .err()
                .map(|err| format!("{err:#}"));
            sender.input(AccountMsg::MicrosoftCredentialSaved {
                key,
                username,
                uuid,
                error,
            });
        });
    }

    /// 打开登录窗口，先只显示「正在获取登录代码」。
    fn open_login_dialog(&mut self, sender: &ComponentSender<Self>) {
        let dialog = adw::Dialog::new();
        dialog.set_title("登录 Microsoft 账户");
        dialog.set_content_width(420);
        // 内容从加载提示换成设备代码后高度会变化，让窗口跟着内容走，
        // 不要留下固定高度带来的空白。
        dialog.set_follows_content_size(true);

        relm4::view! {
            toolbar = adw::ToolbarView {
                add_top_bar = &adw::HeaderBar {},

                #[wrap(Some)]
                set_content = &gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 12,
                    set_valign: gtk::Align::Center,
                    set_margin_top: 24,
                    set_margin_bottom: 24,
                    set_margin_start: 18,
                    set_margin_end: 18,

                    gtk::Spinner {
                        set_spinning: true,
                        set_halign: gtk::Align::Center,
                    },

                    gtk::Label {
                        set_label: "正在获取登录代码…",
                        add_css_class: "dim-label",
                    },
                },
            }
        }

        dialog.set_child(Some(&toolbar));

        let sender = sender.clone();
        dialog.connect_closed(move |_| sender.input(AccountMsg::LoginDialogClosed));

        self.login_dialog = Some(dialog.clone());
        self.login_content = Some(toolbar);
        let parent = relm4::main_adw_application().active_window();
        dialog.present(parent.as_ref());
    }

    /// 拿到设备代码后把加载提示替换成验证说明。
    fn show_login_prompt(&self, prompt: &DeviceCodePrompt) {
        if let Some(toolbar) = &self.login_content {
            toolbar.set_content(Some(&prompt_content(prompt)));
        }
    }

    fn close_login_prompt(&mut self) {
        self.login_content = None;
        if let Some(dialog) = self.login_dialog.take() {
            dialog.close();
        }
    }

    /// 用户关掉提示框：中止后台登录并恢复按钮。
    fn cancel_login(&mut self) {
        self.login_generation = self.login_generation.wrapping_add(1);
        if let Some(task) = self.login_task.take() {
            task.abort();
        }
        self.login_content = None;
        self.logging_in = false;
        self.login_button.set_sensitive(true);
    }
}

/// 设备代码说明页：所有文字居中，底部是复制与打开按钮。
fn prompt_content(prompt: &DeviceCodePrompt) -> gtk::Box {
    let uri = prompt.verification_uri.clone();
    let user_code = prompt.user_code.clone();
    let open_uri = prompt.verification_uri.clone();
    let copy_code = prompt.user_code.clone();
    let expires = format!(
        "代码 {} 秒内有效，登录完成后会自动继续。",
        prompt.expires_in
    );

    relm4::view! {
        content = gtk::Box {
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 12,
            set_valign: gtk::Align::Center,
            set_margin_top: 18,
            set_margin_bottom: 18,
            set_margin_start: 18,
            set_margin_end: 18,

            gtk::Label {
                set_label: "请在浏览器打开下面的验证页面，并输入代码。",
                set_justify: gtk::Justification::Center,
                set_wrap: true,
            },

            gtk::Label {
                set_label: &uri,
                set_justify: gtk::Justification::Center,
                set_selectable: true,
                set_wrap: true,
                add_css_class: "dim-label",
            },

            gtk::Label {
                set_label: &user_code,
                set_selectable: true,
                add_css_class: "title-1",
                add_css_class: "monospace",
            },

            gtk::Label {
                set_label: &expires,
                set_justify: gtk::Justification::Center,
                set_wrap: true,
                add_css_class: "caption",
                add_css_class: "dim-label",
            },

            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 8,
                set_halign: gtk::Align::Center,

                gtk::Button::with_label("复制代码") {
                    add_css_class: "pill",
                    connect_clicked[copy_code] => move |_| {
                        if let Some(display) = gtk::gdk::Display::default() {
                            display.clipboard().set_text(&copy_code);
                        }
                    },
                },

                gtk::Button::with_label("打开验证页面") {
                    add_css_class: "pill",
                    add_css_class: "suggested-action",
                    connect_clicked[open_uri] => move |_| {
                        let launcher = gtk::UriLauncher::new(&open_uri);
                        launcher.launch(
                            relm4::main_adw_application().active_window().as_ref(),
                            None::<&gtk::gio::Cancellable>,
                            |_| {},
                        );
                    },
                },
            },
        }
    }

    content
}

/// 把设备代码交给主线程上的页面组件。
struct GuiPresenter {
    sender: ComponentSender<AccountPage>,
    generation: u64,
}

impl DeviceCodePresenter for GuiPresenter {
    fn present(&self, prompt: &DeviceCodePrompt) {
        self.sender.input(AccountMsg::MicrosoftPrompt {
            generation: self.generation,
            prompt: prompt.clone(),
        });
    }
}

/// 一个账户行：左侧占位头像，标题是玩家名，下方小字是账户类型。
/// 行尾是「设为默认」和删除按钮。
pub(super) struct AccountRow {
    key: String,
    username: String,
    type_label: &'static str,
    avatar: String,
    is_default: bool,
}

/// 账户行发出的动作；带配置键，列表顺序变化也不会指错。
#[derive(Debug)]
pub(super) enum AccountRowOut {
    SetDefault(String),
    Remove(String),
}

#[relm4::factory(pub(super))]
impl FactoryComponent for AccountRow {
    type Init = (String, AccountConfig, bool);
    type Input = bool;
    type Output = AccountRowOut;
    type CommandOutput = ();
    type ParentWidget = adw::PreferencesGroup;
    type Index = DynamicIndex;

    view! {
        adw::ActionRow {
            set_title: &self.username,
            set_subtitle: self.type_label,
            set_subtitle_lines: 1,
            set_subtitle_selectable: false,
            set_activatable: false,

            add_prefix = &adw::Avatar {
                set_size: 40,
                set_show_initials: true,
                set_text: Some(&self.avatar),
                set_valign: gtk::Align::Center,
            },

            #[name(default_button)]
            add_suffix = &gtk::Button {
                #[watch]
                set_icon_name: if self.is_default { "starred-symbolic" } else { "non-starred-symbolic" },
                #[watch]
                set_tooltip_text: Some(if self.is_default { "默认账户" } else { "设为默认" }),
                add_css_class: "flat",
                set_valign: gtk::Align::Center,
            },

            #[name(remove_button)]
            add_suffix = &gtk::Button {
                set_icon_name: "user-trash-symbolic",
                set_tooltip_text: Some("删除"),
                add_css_class: "flat",
                add_css_class: "error",
                set_valign: gtk::Align::Center,
            },
        }
    }

    fn init_model(
        (key, account, is_default): Self::Init,
        _index: &DynamicIndex,
        _sender: FactorySender<Self>,
    ) -> Self {
        let username = account_username(&account).to_string();
        Self {
            key,
            avatar: account_avatar_text(&username),
            username,
            type_label: account_type_label(&account),
            is_default,
        }
    }

    fn init_widgets(
        &mut self,
        _index: &DynamicIndex,
        _root: Self::Root,
        _returned_widget: &gtk::Widget,
        sender: FactorySender<Self>,
    ) -> Self::Widgets {
        let widgets = view_output!();

        widgets.default_button.connect_clicked({
            let sender = sender.clone();
            let key = self.key.clone();
            move |_| {
                let _ = sender.output(AccountRowOut::SetDefault(key.clone()));
            }
        });
        widgets.remove_button.connect_clicked({
            let sender = sender.clone();
            let key = self.key.clone();
            move |_| {
                let _ = sender.output(AccountRowOut::Remove(key.clone()));
            }
        });

        widgets
    }

    fn update(&mut self, is_default: bool, _sender: FactorySender<Self>) {
        self.is_default = is_default;
    }
}

/// 添加离线账户的对话框；只收集玩家名，内部键由页面按重名规则生成。
fn show_add_offline(sender: &ComponentSender<AccountPage>) {
    let title = "添加离线账户";
    let dialog = adw::Dialog::new();
    dialog.set_title(title);
    dialog.set_content_width(360);
    dialog.set_content_height(220);

    relm4::view! {
        toolbar = adw::ToolbarView {
            add_top_bar = &adw::HeaderBar {
                set_show_start_title_buttons: false,
                set_show_end_title_buttons: false,
                set_title_widget: Some(&adw::WindowTitle::new(title, "")),

                pack_start = &gtk::Button::with_label("取消") {
                    connect_clicked[dialog] => move |_| {
                        dialog.close();
                    },
                },

                pack_end: add_button = &gtk::Button::with_label("添加") {
                    add_css_class: "suggested-action",
                },
            },

            #[wrap(Some)]
            set_content = &adw::PreferencesPage {
                add = &adw::PreferencesGroup {
                    add: name_row = &adw::EntryRow {
                        set_title: "玩家名",
                    },
                },
            },
        }
    }

    dialog.set_child(Some(&toolbar));

    let submit = {
        let dialog = dialog.clone();
        let name_row = name_row.clone();
        let sender = sender.clone();
        Rc::new(move || {
            let username = name_row.text().trim().to_string();
            if username.is_empty() {
                return;
            }
            dialog.close();
            sender.input(AccountMsg::SaveOffline(username));
        })
    };

    name_row.connect_changed({
        let name_row = name_row.clone();
        let add_button = add_button.clone();
        move |_| add_button.set_sensitive(!name_row.text().trim().is_empty())
    });
    add_button.connect_clicked({
        let submit = Rc::clone(&submit);
        move |_| submit()
    });
    name_row.connect_apply({
        let submit = Rc::clone(&submit);
        move |_| submit()
    });
    add_button.set_sensitive(false);

    let parent = relm4::main_adw_application().active_window();
    dialog.present(parent.as_ref());
}

/// 删除前的确认框；微软账户会说明凭据也会一起清理。
fn confirm_remove(key: &str, username: &str, sender: &ComponentSender<AccountPage>) {
    let title = format!("删除账户「{username}」？");
    let dialog = adw::AlertDialog::new(Some(&title), None);
    dialog.add_response("cancel", "取消");
    dialog.add_response("delete", "删除");
    dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");

    let sender = sender.clone();
    let key = key.to_string();
    dialog.connect_response(None, move |_, response| {
        if response == "delete" {
            sender.input(AccountMsg::RemoveConfirmed(key.clone()));
        }
    });

    let parent = relm4::main_adw_application().active_window();
    dialog.present(parent.as_ref());
}
