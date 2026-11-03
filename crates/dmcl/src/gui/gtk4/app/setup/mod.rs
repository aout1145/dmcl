//! 首次启动的设置向导。
//!
//! 配置文件不存在时由根组件启动，依次完成账户、Minecraft 目录和版本隔离。
//! 账户页与目录页直接复用首选项里的组件；向导只负责步骤、完成条件和保存。

use std::cell::Cell;
use std::rc::Rc;

use relm4::adw;
use relm4::adw::prelude::*;
use relm4::gtk;
use relm4::gtk::glib;
use relm4::prelude::*;

use crate::config::Config;
use crate::gui::gtk4::pages::settings::{AccountPage, DirectoryPage};

use super::{config_store, ConfigRef, Shell};

/// 向导的三步。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SetupStep {
    Account,
    Directory,
    Game,
}

impl SetupStep {
    fn next(self) -> Option<Self> {
        match self {
            Self::Account => Some(Self::Directory),
            Self::Directory => Some(Self::Game),
            Self::Game => None,
        }
    }

    fn previous(self) -> Option<Self> {
        match self {
            Self::Account => None,
            Self::Directory => Some(Self::Account),
            Self::Game => Some(Self::Directory),
        }
    }

    /// 在内容 `GtkStack` 里的子页名字。
    fn stack_name(self) -> &'static str {
        match self {
            Self::Account => "account",
            Self::Directory => "directory",
            Self::Game => "game",
        }
    }
}

/// 完成向导前还缺什么；`None` 表示三步都已完成。
fn finish_block_reason(config: &Config) -> Option<&'static str> {
    let account = config
        .launch
        .account
        .as_deref()
        .is_some_and(|name| config.accounts.contains_key(name));
    if !account {
        return Some("请先在「账户」中添加账户，并选择一个默认账户。");
    }
    if config.default_root_path().is_err() {
        return Some("请选择一个有效的 Minecraft 目录。");
    }
    None
}

/// 账户步骤使用的默认账户是否有效。
fn has_default_account(config: &Config) -> bool {
    config
        .launch
        .account
        .as_deref()
        .is_some_and(|name| config.accounts.contains_key(name))
}

/// 向导发出的结果。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SetupOut {
    /// 三步都完成，配置已保存。
    Finished,
    /// 用户确认跳过，配置已保存。
    Skipped,
    /// 对话框已经完全关闭，控制器可以丢弃了。
    Closed,
}

/// 向导收到的消息。
#[derive(Debug)]
pub(crate) enum SetupMsg {
    /// 共享配置变了：刷新完成条件和版本隔离开关。
    ConfigChanged,
    Next,
    Back,
    /// 打开「确认跳过」的对话框。
    Skip,
    /// 确认跳过。
    SkipConfirmed,
    VersionIsolationChanged(bool),
    /// 配置保存结束。
    Saved {
        output: SetupOut,
        result: Result<(), String>,
    },
    /// 修改配置时落盘失败。
    SaveFailed(String),
    /// 对话框已完全关闭。
    Closed,
}

pub(crate) struct SetupWizard {
    /// 组件本身就是那个模态对话框。
    dialog: adw::Dialog,
    /// 主窗口：跳过确认框 present 在它里面。
    window: adw::ApplicationWindow,
    config: ConfigRef,
    step: SetupStep,
    /// 账户 / 目录 / 游戏三页的容器；切步骤时切换可见页。
    content_stack: gtk::Stack,
    /// 账户步骤是否已经选了有效账户。
    account_ready: bool,
    /// 完成前还缺什么；`None` 表示可以完成。
    finish_hint: Option<&'static str>,
    /// 最近一次落盘失败，直到用户再次操作才清除。
    error: Option<String>,
    /// 已经保存成功、正在关闭；忽略关闭动画期间的重复点击。
    closing: bool,
    version_isolation: bool,
    version_isolation_row: adw::SwitchRow,
    /// 程序同步开关时屏蔽 `active-notify`。
    syncing: Rc<Cell<bool>>,
    /// 账户与目录页面直接复用首选项里的组件。
    _account: Controller<AccountPage>,
    _directory: Controller<DirectoryPage>,
}

#[relm4::component(pub(crate))]
impl SimpleComponent for SetupWizard {
    type Init = (adw::ApplicationWindow, Rc<Shell>);
    type Input = SetupMsg;
    type Output = SetupOut;

    view! {
        adw::Dialog {
            set_title: "设置向导",
            // 对话框不能拖动改尺寸；主窗口更小时会自动缩小。
            set_content_width: 640,
            set_content_height: 600,
            // 关闭按钮与 Esc 都不能绕过「跳过」确认。
            set_can_close: false,

            // `can-close=false` 时，Esc 等关闭尝试也走「跳过」确认。
            connect_close_attempt[sender] => move |_| {
                sender.input(SetupMsg::Skip);
            },

            // 完全关闭后再让根组件丢弃控制器，避免关闭动画期间还点到旧按钮。
            connect_closed[sender] => move |_| {
                sender.input(SetupMsg::Closed);
            },

            #[wrap(Some)]
            set_child = &adw::ToolbarView {
                add_top_bar = &adw::HeaderBar {
                    set_show_start_title_buttons: false,
                    set_show_end_title_buttons: false,
                    set_title_widget: Some(&adw::WindowTitle::new("设置向导", "")),
                },

                #[wrap(Some)]
                set_content = &gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,

                    #[local_ref]
                    content_stack -> gtk::Stack {
                        set_vexpand: true,
                        // 按子页顺序自动决定左滑 / 右滑方向。
                        set_transition_type: gtk::StackTransitionType::SlideLeftRight,
                    },
                },

                add_bottom_bar = &gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 6,
                    set_margin_top: 6,
                    set_margin_bottom: 12,
                    set_margin_start: 18,
                    set_margin_end: 18,

                    gtk::Label {
                        #[watch]
                        set_visible: model.error.is_some(),
                        #[watch]
                        set_label: model.error.as_deref().unwrap_or(""),
                        set_xalign: 0.0,
                        set_wrap: true,
                        add_css_class: "caption",
                        add_css_class: "error",
                    },

                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 6,

                        gtk::Button::with_label("跳过") {
                            add_css_class: "flat",
                            #[watch]
                            set_visible: model.step != SetupStep::Game,
                            connect_clicked[sender] => move |_| sender.input(SetupMsg::Skip),
                        },

                        gtk::Box {
                            set_hexpand: true,
                        },

                        gtk::Button::with_label("返回") {
                            add_css_class: "flat",
                            #[watch]
                            set_visible: model.step.previous().is_some(),
                            connect_clicked[sender] => move |_| sender.input(SetupMsg::Back),
                        },

                        gtk::Button::with_label("下一步") {
                            add_css_class: "suggested-action",
                            #[watch]
                            set_label: if model.step == SetupStep::Game { "完成" } else { "下一步" },
                            #[watch]
                            set_sensitive: model.can_advance(),
                            connect_clicked[sender] => move |_| sender.input(SetupMsg::Next),
                        },
                    },
                },
            },
        }
    }

    fn init(
        (window, shell): Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let config = config_store::store().config();

        // 账户页与目录页都是现成的组件，直接作为向导的两步。
        let account = AccountPage::builder().launch(shell.clone()).detach();
        let directory = DirectoryPage::builder().launch(shell).detach();

        let (version_isolation, finish_hint) = {
            let config = config.read();
            (
                config.launch.version_isolation,
                finish_block_reason(&config),
            )
        };
        let account_ready = has_default_account(&config.read());

        let syncing = Rc::new(Cell::new(false));
        let version_isolation_row = adw::SwitchRow::builder()
            .title("版本隔离")
            .subtitle("每个版本使用独立的游戏目录；关闭后共用 Minecraft 根目录")
            .active(version_isolation)
            .build();
        version_isolation_row.connect_active_notify({
            let sender = sender.clone();
            let syncing = Rc::clone(&syncing);
            move |row| {
                if syncing.get() {
                    return;
                }
                sender.input(SetupMsg::VersionIsolationChanged(row.is_active()));
            }
        });

        let game_group = adw::PreferencesGroup::builder()
            .title("游戏设置")
            .description("这些设置以后可以在首选项里修改。")
            .build();
        game_group.add(&version_isolation_row);
        let game_page = adw::PreferencesPage::new();
        game_page.add(&game_group);

        let content_stack = gtk::Stack::new();
        content_stack.set_transition_type(gtk::StackTransitionType::SlideLeftRight);
        content_stack.add_named(account.widget(), Some("account"));
        content_stack.add_named(directory.widget(), Some("directory"));
        content_stack.add_named(&game_page, Some("game"));
        content_stack.set_visible_child_name("account");

        let model = SetupWizard {
            dialog: root.clone(),
            window,
            config,
            step: SetupStep::Account,
            content_stack: content_stack.clone(),
            account_ready,
            finish_hint,
            error: None,
            closing: false,
            version_isolation,
            version_isolation_row,
            syncing,
            _account: account,
            _directory: directory,
        };
        let widgets = view_output!();

        // 主窗口和别的启动期提示都先显示，再把模态向导叠上来。
        let dialog = root.clone();
        let window = model.window.clone();
        glib::idle_add_local_once(move || dialog.present(Some(&window)));

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>) {
        match message {
            SetupMsg::ConfigChanged => self.reload(),
            SetupMsg::Next => {
                if self.closing {
                    return;
                }
                if self.step == SetupStep::Game {
                    self.finish(&sender);
                    return;
                }
                if self.can_advance() {
                    self.error = None;
                    let next = self.step.next().expect("前面已经排除了最后一步");
                    self.go_to(next);
                }
            }
            SetupMsg::Back => {
                if self.closing {
                    return;
                }
                self.error = None;
                if let Some(previous) = self.step.previous() {
                    self.go_to(previous);
                }
            }
            SetupMsg::Skip => {
                if !self.closing {
                    self.confirm_skip(&sender);
                }
            }
            SetupMsg::SkipConfirmed => {
                if !self.closing {
                    self.save(&sender, SetupOut::Skipped);
                }
            }
            SetupMsg::VersionIsolationChanged(value) => {
                if self.closing || value == self.version_isolation {
                    return;
                }
                self.error = None;
                self.version_isolation = value;
                config_store::store().update(
                    move |config| config.launch.version_isolation = value,
                    {
                        let sender = sender.clone();
                        move |err| sender.input(SetupMsg::SaveFailed(format!("{err:#}")))
                    },
                );
            }
            SetupMsg::Saved { output, result } => match result {
                Ok(()) => {
                    if self.closing {
                        return;
                    }
                    self.closing = true;
                    // 先上报结果再关闭：根组件处理完结果时，关闭动画可能还没结束，
                    // 因此控制器要等 `Closed` 再丢弃。
                    let _ = sender.output(output);
                    // `can-close=false` 时 `close()` 会被拒绝；保存成功后先放开再正常关闭。
                    self.dialog.set_can_close(true);
                    self.dialog.close();
                }
                Err(err) => self.error = Some(format!("保存配置失败：{err}")),
            },
            SetupMsg::SaveFailed(err) => {
                self.error = Some(format!("保存配置失败：{err}"));
            }
            SetupMsg::Closed => {
                let _ = sender.output(SetupOut::Closed);
            }
        }
    }
}

impl SetupWizard {
    /// 从共享配置重新读取完成条件和版本隔离。
    fn reload(&mut self) {
        let (version_isolation, account_ready, finish_hint) = {
            let config = self.config.read();
            (
                config.launch.version_isolation,
                has_default_account(&config),
                finish_block_reason(&config),
            )
        };
        self.version_isolation = version_isolation;
        self.account_ready = account_ready;
        self.finish_hint = finish_hint;
        self.syncing.set(true);
        self.version_isolation_row.set_active(version_isolation);
        self.syncing.set(false);
    }

    /// 切换步骤：更新内容页。
    fn go_to(&mut self, step: SetupStep) {
        self.step = step;
        self.content_stack.set_visible_child_name(step.stack_name());
    }

    /// 当前步骤是否可以继续。
    fn can_advance(&self) -> bool {
        match self.step {
            SetupStep::Account => self.account_ready,
            SetupStep::Directory => true,
            SetupStep::Game => self.finish_hint.is_none(),
        }
    }

    /// 点「完成」：完成按钮已经由 `finish_hint` 控制可用状态。
    fn finish(&mut self, sender: &ComponentSender<Self>) {
        if self.finish_hint.is_some() {
            return;
        }
        self.error = None;
        self.save(sender, SetupOut::Finished);
    }

    /// 先把当前配置落盘，成功后才关闭对话框并上报结果。
    fn save(&self, sender: &ComponentSender<Self>, output: SetupOut) {
        let sender = sender.clone();
        config_store::store().save(move |result| {
            sender.input(SetupMsg::Saved {
                output,
                result: result.map_err(|err| format!("{err:#}")),
            });
        });
    }

    /// 「跳过」先确认；确认后同样会保存配置，之后不再弹向导。
    fn confirm_skip(&self, sender: &ComponentSender<Self>) {
        let dialog = adw::AlertDialog::new(
            Some("设置尚未完成，确认跳过？"),
            Some("以后可以在首选项里修改这些设置。"),
        );
        dialog.add_response("continue", "继续设置");
        dialog.add_response("skip", "跳过");
        dialog.set_default_response(Some("continue"));
        dialog.set_close_response("continue");

        let sender = sender.clone();
        dialog.connect_response(None, move |_, response| {
            if response == "skip" {
                sender.input(SetupMsg::SkipConfirmed);
            }
        });

        dialog.present(Some(&self.window));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AccountConfig;

    #[test]
    fn finish_requires_a_valid_default_account_and_root() {
        let mut config = Config::default();
        assert!(finish_block_reason(&config).is_some());

        config
            .add_account(
                "player",
                AccountConfig::Offline {
                    username: "Player".to_string(),
                },
            )
            .unwrap();
        assert!(finish_block_reason(&config).is_none());

        config.launch.account = None;
        assert!(finish_block_reason(&config).is_some());

        config.launch.account = Some("missing".to_string());
        assert!(finish_block_reason(&config).is_some());

        config.launch.account = Some("player".to_string());
        config.minecraft_root = "missing-root".to_string();
        assert!(finish_block_reason(&config).is_some());
    }
}
