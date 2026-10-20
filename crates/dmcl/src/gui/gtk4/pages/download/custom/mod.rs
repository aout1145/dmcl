//! 「下载 / 自定义」页：新建实例。
//!
//! 页面由两块 `download` 公共组件组装：
//!
//! * [`InstanceSummary`]：图标 + 名称（自动命名 / 用户改过后不再覆盖）+ 分类（v1 禁用）
//!   + 安装按钮；
//! * 四个 [`VersionSelect`]：Minecraft、Forge、Fabric、NeoForge。加载器列表随所选 MC
//!   版本动态取；「最新 / 推荐 / 稳定」的判定与徽标在下面的适配函数里做，组件本身只
//!   展示。
//!
//! 加载器互斥按**组**配置（[`LoaderDef::mutex_group`]），现阶段 Forge / Fabric /
//! NeoForge 同属 `"loader"` 组，所以选了一个会静默清掉同组其它项。以后要允许叠加
//! （如双加载器整合包）只需改表里的组。
//!
//! 安装成功后页面关闭自己（[`TabTitle::close`]），标签的移除走根组件的 `close-page`。

use std::rc::Rc;
use std::sync::Arc;

use relm4::gtk;
use relm4::gtk::prelude::*;
use relm4::prelude::*;

use dmcl_install::{default_version_name, InstallSpec, LoaderKind};
use dmcl_mcmeta::source::DownloadSource;

use crate::config::Config;
use crate::gui::gtk4::app::page_host::{Page, PageInit, TabTitle};
use crate::gui::gtk4::app::Shell;
use crate::gui::gtk4::pages::download::common::instance_summary::{
    IconSource, InstanceSummary, SummaryIn, SummaryOut,
};
use crate::gui::gtk4::pages::download::common::version_select::{
    Badge, FetchVersions, KindChip, SelectIn, SelectOut, VersionItem, VersionSelect,
    VersionSelectInit,
};

/// 一个加载器下拉的静态定义。
struct LoaderDef {
    kind: LoaderKind,
    /// 互斥组：同组里只能选一个（`None` = 不参与互斥）。
    ///
    /// 现在三个加载器同属 `"loader"`；将来若要允许叠加（比如双加载器整合包），改这里
    /// 的组名即可，页面逻辑不用动。
    mutex_group: Option<&'static str>,
}

/// 加载器下拉表（也是页面上的显示顺序）。
const LOADER_DEFS: &[LoaderDef] = &[
    LoaderDef {
        kind: LoaderKind::Forge,
        mutex_group: Some("loader"),
    },
    LoaderDef {
        kind: LoaderKind::Fabric,
        mutex_group: Some("loader"),
    },
    LoaderDef {
        kind: LoaderKind::NeoForge,
        mutex_group: Some("loader"),
    },
];

/// 页面消息。
#[derive(Debug)]
pub(crate) enum CustomMsg {
    /// Minecraft 版本下拉的选择变化。
    McChanged(Option<VersionItem>),
    /// 第 `slot` 个加载器下拉的选择变化。
    LoaderChanged {
        slot: usize,
        item: Option<VersionItem>,
    },
    /// 实例名称显示值变化（自动命名 / 用户编辑都会发）。
    NameChanged(String),
    /// 用户点了安装按钮。
    Install { name: String },
    /// 安装任务结束（成功关页，失败留在页面上）。
    Installed {
        name: String,
        result: Result<(), String>,
    },
}

/// 一个加载器下拉 + 它当前的选择。
struct LoaderSlot {
    kind: LoaderKind,
    mutex_group: Option<&'static str>,
    select: Controller<VersionSelect>,
    selection: Option<VersionItem>,
}

pub(crate) struct CustomPage {
    /// 标签标题句柄：改标题、安装成功后关页都走它。
    tab: TabTitle,
    shell: Rc<Shell>,
    config: Config,
    summary: Controller<InstanceSummary>,
    /// MC 版本下拉（只保活，页面不再向它发消息）。
    _mc: Controller<VersionSelect>,
    loaders: Vec<LoaderSlot>,
    mc_version: Option<VersionItem>,
    installing: bool,
}

impl CustomPage {
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
                    .map(|item| (slot.kind, Some(item.id.as_str())))
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
        let _ = self.summary.sender().send(SummaryIn::SetInstallEnabled(
            self.mc_version.is_some() && !self.installing,
        ));
    }

    /// 安装当前装配。
    fn install(&mut self, name: String, sender: ComponentSender<Self>) {
        let Some(mc) = self.mc_version.clone() else {
            self.shell.toast("请先选择 Minecraft 版本");
            return;
        };
        if name.trim().is_empty() {
            self.shell.toast("实例名称不能为空");
            return;
        }

        self.installing = true;
        let _ = self
            .summary
            .sender()
            .send(SummaryIn::SetInstallEnabled(false));

        let mut spec = InstallSpec::new(mc.id, name.clone());
        for slot in &self.loaders {
            if let Some(item) = &slot.selection {
                spec = spec.with_loader(slot.kind, item.id.clone());
            }
        }
        let options = Arc::new(self.config.install_options(self.config.download_source()));
        let handle = self.shell.run(
            format!("安装 {name}"),
            dmcl_install::install_task(options, spec),
            (),
        );

        let sender = sender.clone();
        relm4::spawn(async move {
            let result = handle.finished().await.map(|_| ());
            sender.input(CustomMsg::Installed { name, result });
        });
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
                    KindChip::new("release", "正式版"),
                    KindChip::new("snapshot", "快照"),
                    KindChip::new("old", "远古"),
                ],
                allow_none: false,
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
            installing: false,
        };
        model.tab.set_title("新实例");
        let _ = model
            .summary
            .sender()
            .send(SummaryIn::SetInstallEnabled(false));

        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>) {
        match message {
            CustomMsg::McChanged(item) => {
                self.mc_version = item;
                // 加载器列表随 MC 版本重取；旧选择失效（组件自己发 None 时会再进来一次）。
                let context = self.mc_version.as_ref().map(|item| item.id.clone());
                for slot in &mut self.loaders {
                    slot.selection = None;
                    let _ = slot
                        .select
                        .sender()
                        .send(SelectIn::SetContext(context.clone()));
                }
                self.refresh_auto();
            }
            CustomMsg::LoaderChanged { slot, item } => {
                // 互斥：同组里选了新的，静默清掉其它（不发输出，避免来回打转）。
                if item.is_some() {
                    let group = self.loaders[slot].mutex_group;
                    if group.is_some() {
                        let mut cleared = Vec::new();
                        for (index, other) in self.loaders.iter().enumerate() {
                            if index != slot && other.mutex_group == group {
                                let _ = other.select.sender().send(SelectIn::Clear);
                                cleared.push(index);
                            }
                        }
                        for index in cleared {
                            self.loaders[index].selection = None;
                        }
                    }
                }
                self.loaders[slot].selection = item;
                self.refresh_auto();
            }
            CustomMsg::NameChanged(name) => {
                if name.trim().is_empty() {
                    self.tab.set_title("新实例");
                } else {
                    self.tab.set_title(&name);
                }
            }
            CustomMsg::Install { name } => self.install(name, sender),
            CustomMsg::Installed { name, result } => {
                self.installing = false;
                self.refresh_auto();
                match result {
                    Ok(()) => {
                        self.shell.toast(&format!("{name} 安装完成"));
                        // 安装成功自动关页：走 close-page，根组件对非主页放行。
                        self.tab.close();
                    }
                    Err(err) => self.shell.toast(&format!("安装失败：{err}")),
                }
            }
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

/// 加载器下拉的 Init（取数器捕获下载源，按上下文取列表）。
fn loader_init(kind: LoaderKind, source: DownloadSource) -> VersionSelectInit {
    match kind {
        // Forge：列表新→旧；推荐版本来自 `branch == "recommended"`。
        // 稳定 / 推荐的筛选项留给后续（TODO：chips），目前只有搜索。
        LoaderKind::Forge => VersionSelectInit {
            title: format!("{} 版本", kind.display_name()),
            fetch: Arc::new(move |context: &str| {
                let source = source.clone();
                let context = context.to_string();
                Box::pin(async move {
                    let entries = source.fetch_forge_versions(&context).await?;
                    Ok(forge_items(entries))
                })
            }),
            chips: Vec::new(),
            allow_none: true,
        },
        // Fabric：稳定 / 不稳定两个筛选；置顶「最新」+「稳定」。
        LoaderKind::Fabric => VersionSelectInit {
            title: format!("{} 版本", kind.display_name()),
            fetch: Arc::new(move |context: &str| {
                let source = source.clone();
                let context = context.to_string();
                Box::pin(async move {
                    let entries = source.fetch_fabric_loaders(&context).await?;
                    Ok(fabric_items(entries))
                })
            }),
            chips: vec![
                KindChip::new("stable", "稳定"),
                KindChip::new("unstable", "不稳定"),
            ],
            allow_none: true,
        },
        // NeoForge：只有 1.20.1 之后的版本，空列表是常态（Empty 状态）。
        LoaderKind::NeoForge => VersionSelectInit {
            title: format!("{} 版本", kind.display_name()),
            fetch: Arc::new(move |context: &str| {
                let source = source.clone();
                let context = context.to_string();
                Box::pin(async move {
                    let entries = source.fetch_neoforge_versions(&context).await?;
                    Ok(neoforge_items(entries))
                })
            }),
            chips: Vec::new(),
            allow_none: true,
        },
    }
}

/// Minecraft 版本清单的取数器：**忽略上下文**，一次取全清单。
fn mc_fetcher(source: DownloadSource) -> FetchVersions {
    Arc::new(move |_context: &str| {
        let source = source.clone();
        Box::pin(async move {
            let manifest = source.fetch_version_manifest().await?;
            Ok(mc_items(manifest))
        })
    })
}

fn badge(key: &str, label: &str) -> Badge {
    Badge {
        key: key.to_string(),
        label: label.to_string(),
    }
}

/// 原版清单 → 条目：`latest.release` / `latest.snapshot` 置顶，类型合并「远古」。
fn mc_items(manifest: dmcl_mcmeta::vanilla::manifest::VersionManifest) -> Vec<VersionItem> {
    let latest_release = manifest
        .latest
        .as_ref()
        .map(|latest| latest.release.clone())
        .unwrap_or_default();
    let latest_snapshot = manifest
        .latest
        .as_ref()
        .map(|latest| latest.snapshot.clone())
        .unwrap_or_default();

    manifest
        .versions
        .iter()
        .map(|entry| {
            // old_alpha / old_beta（以及部分镜像的 alpha / beta）统一并成「远古」，
            // 免得筛选 chips 跟着远程数据的措辞变化。
            let kind = match entry.kind.as_str() {
                "old_alpha" | "old_beta" | "alpha" | "beta" => "old",
                other => other,
            };
            let mut badges = Vec::new();
            if entry.id == latest_release {
                badges.push(badge("latest", "最新"));
            } else if entry.id == latest_snapshot {
                badges.push(badge("latest", "最新快照"));
            }
            VersionItem {
                id: entry.id.clone(),
                label: entry.id.clone(),
                kind: kind.to_string(),
                badges,
                detail: entry
                    .release_time
                    .as_deref()
                    .map(|time| time.chars().take(10).collect()),
            }
        })
        .collect()
}

/// Forge 列表 → 条目：第一条「最新」，`branch == "recommended"` 的「推荐」。
fn forge_items(entries: Vec<dmcl_mcmeta::forge::ForgeVersionEntry>) -> Vec<VersionItem> {
    let first = entries.first().map(|entry| entry.full_name());
    let recommended = entries
        .iter()
        .find(|entry| entry.branch.as_deref() == Some("recommended"))
        .map(|entry| entry.full_name());

    entries
        .into_iter()
        .map(|entry| {
            let full_name = entry.full_name();
            let mut badges = Vec::new();
            if first.as_deref() == Some(full_name.as_str()) {
                badges.push(badge("latest", "最新"));
            }
            if recommended.as_deref() == Some(full_name.as_str()) {
                badges.push(badge("recommended", "推荐"));
            }
            VersionItem {
                id: full_name.clone(),
                label: full_name,
                kind: String::new(),
                badges,
                detail: Some(format!("构建 {}", entry.build)),
            }
        })
        .collect()
}

/// Fabric 列表 → 条目：第一条稳定版「最新」+「稳定」（没有稳定版才退回第一条）；
/// 按 `stable` 分类。
fn fabric_items(entries: Vec<dmcl_mcmeta::fabric::FabricLoaderEntry>) -> Vec<VersionItem> {
    let first_stable = entries
        .iter()
        .position(|entry| entry.loader.stable == Some(true));
    // CLI 的 latest 语义：优先稳定版，找不到稳定版才退回列表第一条。
    let latest = first_stable.unwrap_or(0);

    entries
        .into_iter()
        .enumerate()
        .map(|(index, entry)| {
            let stable = entry.loader.stable == Some(true);
            let mut badges = Vec::new();
            if index == latest {
                badges.push(badge("latest", "最新"));
            }
            if first_stable == Some(index) {
                badges.push(badge("stable", "稳定"));
            }
            VersionItem {
                id: entry.loader.version.clone(),
                label: entry.loader.version,
                kind: if stable { "stable" } else { "unstable" }.to_string(),
                badges,
                detail: None,
            }
        })
        .collect()
}

/// NeoForge 列表 → 条目：第一条「最新」（已按版本号新→旧排序）。
fn neoforge_items(entries: Vec<dmcl_mcmeta::neoforge::NeoForgeListEntry>) -> Vec<VersionItem> {
    entries
        .into_iter()
        .enumerate()
        .map(|(index, entry)| VersionItem {
            id: entry.version.clone(),
            label: entry.version,
            kind: String::new(),
            badges: if index == 0 {
                vec![badge("latest", "最新")]
            } else {
                Vec::new()
            },
            detail: None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use dmcl_mcmeta::fabric::{FabricLoader, FabricLoaderEntry};
    use dmcl_mcmeta::forge::ForgeVersionEntry;
    use dmcl_mcmeta::neoforge::NeoForgeListEntry;
    use dmcl_mcmeta::vanilla::manifest::{LatestVersions, VersionManifest, VersionManifestEntry};

    fn labels(item: &VersionItem) -> Vec<&str> {
        item.badges
            .iter()
            .map(|badge| badge.label.as_str())
            .collect()
    }

    fn manifest_entry(id: &str, kind: &str) -> VersionManifestEntry {
        VersionManifestEntry {
            id: id.to_string(),
            kind: kind.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn mc_items_merge_old_kinds_and_badge_latest() {
        let manifest = VersionManifest {
            versions: vec![
                manifest_entry("24w00a", "snapshot"),
                manifest_entry("1.21.10", "release"),
                manifest_entry("a1.0", "old_alpha"),
            ],
            latest: Some(LatestVersions {
                release: "1.21.10".to_string(),
                snapshot: "24w00a".to_string(),
            }),
        };

        let items = mc_items(manifest);
        assert_eq!(labels(&items[0]), vec!["最新快照"]);
        assert_eq!(labels(&items[1]), vec!["最新"]);
        // old_alpha / old_beta 合并成 old，供「远古」chip 过滤。
        assert_eq!(items[2].kind, "old");
        assert!(items[2].badges.is_empty());
    }

    #[test]
    fn forge_items_badge_first_and_recommended() {
        let entries = vec![
            ForgeVersionEntry {
                mcversion: "1.20.1".to_string(),
                version: "47.2.0".to_string(),
                build: 3,
                ..Default::default()
            },
            ForgeVersionEntry {
                mcversion: "1.20.1".to_string(),
                version: "47.1.0".to_string(),
                branch: Some("recommended".to_string()),
                build: 2,
                ..Default::default()
            },
        ];

        let items = forge_items(entries);
        assert_eq!(items[0].id, "1.20.1-47.2.0");
        assert_eq!(labels(&items[0]), vec!["最新"]);
        assert_eq!(labels(&items[1]), vec!["推荐"]);
        assert_eq!(items[1].detail.as_deref(), Some("构建 2"));
    }

    fn fabric_entry(version: &str, stable: bool) -> FabricLoaderEntry {
        FabricLoaderEntry {
            loader: FabricLoader {
                version: version.to_string(),
                stable: Some(stable),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn fabric_items_badge_first_stable_as_latest() {
        let items = fabric_items(vec![
            fabric_entry("0.16.0", false),
            fabric_entry("0.15.11", true),
            fabric_entry("0.15.10", true),
        ]);

        // 最新一条不稳定：CLI 语义下「最新」给第一条稳定版，它同时「稳定」。
        assert!(items[0].badges.is_empty());
        assert_eq!(labels(&items[1]), vec!["最新", "稳定"]);
        // 只有第一条稳定版进置顶，其余稳定版不重复。
        assert!(items[2].badges.is_empty());
        assert_eq!(items[0].kind, "unstable");
        assert_eq!(items[1].kind, "stable");
    }

    #[test]
    fn fabric_items_can_carry_both_badges_on_the_first_entry() {
        let items = fabric_items(vec![fabric_entry("0.16.0", true)]);
        assert_eq!(labels(&items[0]), vec!["最新", "稳定"]);
    }

    #[test]
    fn neoforge_items_badge_only_the_first() {
        let items = neoforge_items(vec![
            NeoForgeListEntry {
                version: "21.1.80".to_string(),
                ..Default::default()
            },
            NeoForgeListEntry {
                version: "21.1.79".to_string(),
                ..Default::default()
            },
        ]);

        assert_eq!(labels(&items[0]), vec!["最新"]);
        assert!(items[1].badges.is_empty());
    }
}
