//! 「下载 / 自定义」页：新建实例。
//!
//! 页面由两块 `download` 公共组件组装：
//!
//! * [`InstanceSummary`]：图标 + 名称（自动命名 / 用户改过后不再覆盖）+ 分类（v1 禁用）
//!   + 安装按钮；
//! * 四个 [`VersionSelect`]：Minecraft、Forge、Fabric、NeoForge。加载器列表随所选 MC
//!   版本动态取；「最新 / 推荐」的判定与徽标在 `versions` 子模块里做，组件本身只展示。
//!
//! 加载器互斥按**组**配置（`LoaderDef::mutex_group`），现阶段 Forge / Fabric /
//! NeoForge 同属 `"loader"` 组：选中一个后，同组其它行会被禁用并提示「与 xx 冲突」，
//! 取消选择后恢复。以后要允许叠加（如双加载器整合包）只需改表里的组。
//!
//! 用户点安装后立即关闭本页并主动打开任务列表（[`Shell::open_tasks`]）；任务结果由
//! 任务窗口展示。标签的移除走根组件的 `close-page`。

mod loaders;
mod message;
mod versions;

use std::rc::Rc;
use std::sync::Arc;

use relm4::gtk;
use relm4::gtk::prelude::*;
use relm4::prelude::*;

use dmcl_install::{default_version_name, InstallSpec, LoaderKind};
use dmcl_mcmeta::layout::version_dir;

use crate::config::Config;
use crate::gui::gtk4::app::page_host::{Page, PageInit, TabTitle};
use crate::gui::gtk4::app::Shell;
use crate::gui::gtk4::pages::download::common::instance_summary::{
    IconSource, InstanceSummary, SummaryIn, SummaryOut,
};
use crate::gui::gtk4::pages::download::common::version_select::{
    KindChip, SelectIn, SelectOut, VersionItem, VersionSelect, VersionSelectInit,
};

use loaders::{loader_init, LoaderSlot, LOADER_DEFS};
use message::CustomMsg;
use versions::mc_fetcher;

pub(crate) struct CustomPage {
    /// 标签标题句柄：改标题、安装发起后关页都走它。
    tab: TabTitle,
    shell: Rc<Shell>,
    config: Config,
    summary: Controller<InstanceSummary>,
    /// MC 版本下拉（只保活，页面不再向它发消息）。
    _mc: Controller<VersionSelect>,
    loaders: Vec<LoaderSlot>,
    mc_version: Option<VersionItem>,
    /// 概要组件当前显示的名称（自动 / 用户编辑都由 `NameChanged` 同步）。
    name: String,
}

/// 标签标题：`安装 <实例名>`；还没名字时用「安装 新实例」。
fn install_tab_title(name: &str) -> String {
    if name.trim().is_empty() {
        "安装 新实例".to_string()
    } else {
        format!("安装 {name}")
    }
}

impl CustomPage {
    /// `versions/<name>` 是否已经存在。空名称不算冲突（安装前另有非空校验）。
    ///
    /// 路径用原始名称（和安装器一致），只把全空白当作空；不要 `trim` 后再查，
    /// 否则 `"26.3 "` 会被误判成已有的 `26.3`。
    fn version_exists(&self, name: &str) -> bool {
        let Ok(root) = self.config.minecraft_root_path() else {
            return false;
        };
        !name.trim().is_empty() && version_dir(root, name).exists()
    }

    /// 按同组选择刷新加载器冲突：已选中的行让其它同组行禁用并显示「与 xx 冲突」。
    fn refresh_loader_conflicts(&self) {
        for (index, slot) in self.loaders.iter().enumerate() {
            let conflict = slot.mutex_group.and_then(|group| {
                self.loaders
                    .iter()
                    .enumerate()
                    .find_map(|(other_index, other)| {
                        (other_index != index
                            && other.mutex_group == Some(group)
                            && other.selection.is_some())
                        .then(|| other.kind.display_name().to_string())
                    })
            });
            let _ = slot.select.sender().send(SelectIn::SetConflict(conflict));
        }
    }

    /// 按当前状态同步名称冲突提示 / 安装按钮可用性。
    fn refresh_name_state(&self) {
        let exists = self.version_exists(&self.name);
        let error = exists.then(|| "实例已经存在".to_string());
        let _ = self.summary.sender().send(SummaryIn::SetNameError(error));
        let enabled = self.mc_version.is_some() && !self.name.trim().is_empty() && !exists;
        let _ = self
            .summary
            .sender()
            .send(SummaryIn::SetInstallEnabled(enabled));
    }

    /// 重算自动命名 / 自动图标 / 安装按钮可用性并推给概要组件。
    ///
    /// 用户手动改过名称或图标时，概要组件自己会忽略自动值（它记着「改过没有」）。
    fn refresh_auto(&mut self) {
        let loaders: Vec<(LoaderKind, Option<&str>)> = self
            .loaders
            .iter()
            .filter_map(|slot| {
                slot.selection
                    .as_ref()
                    // 自动名用展示版本：Forge 的 label 是 version 字段（不带 mcversion），
                    // 安装 id 仍是 full_name。
                    .map(|item| (slot.kind, Some(item.label.as_str())))
            })
            .collect();

        match &self.mc_version {
            Some(mc) => {
                let name = default_version_name(&mc.id, &loaders);
                let _ = self.summary.sender().send(SummaryIn::SetAutoName(name));
                let icon = self
                    .loaders
                    .iter()
                    .find(|slot| slot.selection.is_some())
                    .map(|slot| IconSource::Loader(slot.kind))
                    .unwrap_or(IconSource::Initials);
                let _ = self.summary.sender().send(SummaryIn::SetAutoIcon(icon));
            }
            None => {
                // 没选 MC 版本：清掉自动命名，安装不可用。
                let _ = self
                    .summary
                    .sender()
                    .send(SummaryIn::SetAutoName(String::new()));
                let _ = self
                    .summary
                    .sender()
                    .send(SummaryIn::SetAutoIcon(IconSource::Initials));
            }
        }
        self.refresh_name_state();
    }

    /// 安装当前装配：把任务交给任务窗口，然后立即关页并打开任务列表。
    fn install(&mut self, name: String) {
        let Some(mc) = self.mc_version.clone() else {
            self.shell.toast("请先选择 Minecraft 版本");
            return;
        };
        if name.trim().is_empty() {
            self.shell.toast("实例名称不能为空");
            return;
        }
        let mut spec = InstallSpec::new(mc.id, name.clone());
        for slot in &self.loaders {
            if let Some(item) = &slot.selection {
                // 安装用 id（Forge 是 full_name）；label 只用于展示和自动命名。
                spec = spec.with_loader(slot.kind, item.id.clone());
            }
        }
        let options = match self
            .config
            .install_options(self.config.download_source(), &self.config.launch)
        {
            Ok(options) => Arc::new(options),
            Err(err) => {
                self.shell.toast(&format!("无法安装：{err:#}"));
                return;
            }
        };
        // 结果句柄故意丢掉：任务已经在 Shell / 任务窗口那边独立跑，本页马上关闭。
        let _ = self.shell.run(
            format!("安装 {name}"),
            dmcl_install::install_task(options, spec),
            (),
        );
        // 先弹任务窗口，再在下一轮 idle 关页：任务窗口已经在，关页不会把它压到主窗口下面。
        // 关页会丢掉页面本体，所以先用克隆拿住 `TabTitle`，idle 里不再碰 `self`。
        self.shell.open_tasks();
        let tab = self.tab.clone();
        gtk::glib::idle_add_local_once(move || tab.close());
    }
}

#[relm4::component(pub(crate))]
impl SimpleComponent for CustomPage {
    type Init = PageInit<(Rc<Shell>, Config)>;
    type Input = CustomMsg;
    type Output = ();

    view! {
        gtk::ScrolledWindow {
            set_hscrollbar_policy: gtk::PolicyType::Never,
            set_vexpand: true,

            gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 18,
                set_margin_all: 18,

                #[local_ref]
                summary_widget -> gtk::Box {},

                #[local_ref]
                versions_list -> gtk::ListBox {},
            }
        }
    }

    fn init(
        init: Self::Init,
        _root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let PageInit {
            tab,
            args: (shell, config),
        } = init;
        let source = config.download_source();

        let summary = InstanceSummary::builder()
            .launch(())
            .forward(sender.input_sender(), |out| match out {
                SummaryOut::Install { name } => CustomMsg::Install { name },
                SummaryOut::NameChanged(name) => CustomMsg::NameChanged(name),
            });

        let mc = VersionSelect::builder()
            .launch(VersionSelectInit {
                title: "Minecraft 版本".to_string(),
                fetch: mc_fetcher(source.clone()),
                chips: vec![
                    KindChip::new("release", "正式版", true),
                    KindChip::new("snapshot", "快照版", false),
                    KindChip::new("old", "远古版", false),
                ],
                show_search: true,
                allow_none: false,
                initially_expanded: true,
            })
            .forward(sender.input_sender(), |out| {
                CustomMsg::McChanged(take_selection(out))
            });

        // 加载器下拉：按表逐个建，输出带上槽位下标。
        let mut loaders = Vec::new();
        for (slot, def) in LOADER_DEFS.iter().enumerate() {
            let select = VersionSelect::builder()
                .launch(loader_init(def.kind, source.clone()))
                .forward(sender.input_sender(), move |out| CustomMsg::LoaderChanged {
                    slot,
                    item: take_selection(out),
                });
            loaders.push(LoaderSlot {
                kind: def.kind,
                mutex_group: def.mutex_group,
                select,
                selection: None,
            });
        }

        let summary_widget = summary.widget().clone();
        let versions_list = gtk::ListBox::new();
        versions_list.add_css_class("boxed-list");
        versions_list.set_selection_mode(gtk::SelectionMode::None);
        versions_list.append(mc.widget());
        for slot in &loaders {
            versions_list.append(slot.select.widget());
        }

        // 打开页面时：MC 版本清单先取起来（不依赖上下文）。
        let _ = mc.sender().send(SelectIn::SetContext(Some(String::new())));

        let model = CustomPage {
            tab,
            shell,
            config,
            summary,
            _mc: mc,
            loaders,
            mc_version: None,
            name: String::new(),
        };
        model.tab.set_title(&install_tab_title(""));
        model.refresh_name_state();

        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>) {
        match message {
            CustomMsg::McChanged(item) => {
                let old_context = self.mc_version.as_ref().map(|item| item.id.clone());
                self.mc_version = item;
                let context = self.mc_version.as_ref().map(|item| item.id.clone());
                // 只有 MC 上下文真的变了才让加载器选择失效；重复选同一个版本时保留。
                if old_context != context {
                    for slot in &mut self.loaders {
                        slot.selection = None;
                        let _ = slot
                            .select
                            .sender()
                            .send(SelectIn::SetContext(context.clone()));
                    }
                    self.refresh_loader_conflicts();
                }
                self.refresh_auto();
            }
            CustomMsg::LoaderChanged { slot, item } => {
                self.loaders[slot].selection = item;
                // 同组其它行由组件自己禁用；这里只同步「与 xx 冲突」提示。
                self.refresh_loader_conflicts();
                self.refresh_auto();
            }
            CustomMsg::NameChanged(name) => {
                self.name = name;
                self.tab.set_title(&install_tab_title(&self.name));
                self.refresh_name_state();
                // 名称被清空 = 放弃手动命名：重新计算并推自动名，让概要组件重新接管。
                if self.name.trim().is_empty() {
                    self.refresh_auto();
                }
            }
            CustomMsg::Install { name } => self.install(name),
        }
    }
}

impl Page for CustomPage {
    type Args = (Rc<Shell>, Config);
}

/// 从组件输出里取选择（`Changed` 是唯一输出）。
fn take_selection(out: SelectOut) -> Option<VersionItem> {
    match out {
        SelectOut::Changed(item) => item,
    }
}

#[cfg(test)]
mod tests {
    use super::install_tab_title;

    #[test]
    fn tab_title_has_install_prefix() {
        assert_eq!(install_tab_title(""), "安装 新实例");
        assert_eq!(install_tab_title("  "), "安装 新实例");
        assert_eq!(install_tab_title("26.3"), "安装 26.3");
        assert_eq!(
            install_tab_title("26.3-forge-47.2.0"),
            "安装 26.3-forge-47.2.0"
        );
    }
}
