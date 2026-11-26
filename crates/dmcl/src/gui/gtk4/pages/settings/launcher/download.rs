//! 首选项里的「下载」分类：下载源、并发、分块、重试与安装器 Java。
//!
//! 改动先写进本页副本，防抖后整段写回共享配置。下载任务在开始时读取配置，所以修改从下一次
//! 安装或启动起生效。

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use dmcl_java::JavaDetails;
use dmcl_mcmeta::source::MirrorPolicy;
use relm4::adw;
use relm4::adw::prelude::*;
use relm4::gtk;
use relm4::gtk::glib;
use relm4::prelude::*;

use crate::config::DownloadConfig;
use crate::gui::gtk4::app::{config_store, ConfigRef, PageToast, Shell};
use crate::gui::gtk4::pages::settings::common::{ChoiceRowOut, JavaChoices, JavaProbeCache};

/// 连续改动合并成一次写回的间隔。
const FLUSH_DELAY: Duration = Duration::from_millis(400);

/// 镜像策略下拉里的文字；顺序与 [`MirrorPolicy::ALL`] 一致。
const MIRROR_LABELS: [&str; 3] = ["仅官方源", "官方优先", "镜像优先"];

/// 「下载」分类收到的消息。
#[derive(Debug)]
pub(super) enum DownloadMsg {
    /// 页面显示出来了：开始探测还没缓存的 Java。
    Shown,
    /// 页面隐藏了：停止发起新的探测。
    Hidden,
    /// 共享配置变了：没有待写改动时重新读取。
    ConfigChanged,
    MirrorSelected(u32),
    SkipMusicChanged(bool),
    MaxConnectionsChanged(u32),
    MaxActiveFilesChanged(u32),
    MaxWorkersChanged(u32),
    ChunkThresholdChanged(u32),
    ChunkSizeChanged(u32),
    MaxRetriesChanged(u32),
    RetryBaseDelayChanged(u32),
    RetryMaxDelayChanged(u32),
    JavaSelected(u32),
    /// 某个 Java 探测结束。
    Probed {
        path: PathBuf,
        result: Result<JavaDetails, String>,
    },
    /// 防抖到点，写回改动。
    Flush,
}

/// 「下载」分类的初始化参数。
pub(super) struct DownloadPageInit {
    pub shell: Rc<Shell>,
    /// 与 Java 管理页、游戏设置页共享的探测结果缓存。
    pub cache: Rc<JavaProbeCache>,
}

pub(super) struct DownloadPage {
    config: ConfigRef,
    shell: Rc<Shell>,
    /// 页面级提示：只有首选项可见时才显示。
    page_toast: PageToast,
    /// 同步控件时临时屏蔽信号。
    syncing: Rc<Cell<bool>>,
    /// 页面正在编辑的下载配置。
    download: DownloadConfig,
    /// 安装器 Java 的展开列表。
    java_list: JavaChoices,
    /// 是否有还没写回共享配置的改动。
    dirty: bool,
    /// 防抖计时器。
    flush: Option<glib::SourceId>,

    mirror_row: adw::ComboRow,
    skip_music_row: adw::SwitchRow,
    max_connections_row: adw::SpinRow,
    max_active_files_row: adw::SpinRow,
    max_workers_row: adw::SpinRow,
    chunk_threshold_row: adw::SpinRow,
    chunk_size_row: adw::SpinRow,
    max_retries_row: adw::SpinRow,
    retry_base_delay_row: adw::SpinRow,
    retry_max_delay_row: adw::SpinRow,
    java_expander: adw::ExpanderRow,
}

#[relm4::component(pub(super))]
impl SimpleComponent for DownloadPage {
    type Init = DownloadPageInit;
    type Input = DownloadMsg;
    type Output = ();

    view! {
        adw::PreferencesPage {
            add = &adw::PreferencesGroup {
                set_title: "下载源",
                set_description: Some("选择下载 Minecraft 文件时使用的源。"),
                #[local_ref]
                mirror_row -> adw::ComboRow {},
            },

            add = &adw::PreferencesGroup {
                set_title: "下载内容",
                set_description: Some("选择下载时可跳过的内容。"),
                #[local_ref]
                skip_music_row -> adw::SwitchRow {},
            },

            add = &adw::PreferencesGroup {
                set_title: "并发",
                set_description: Some("同时进行的下载越多，占用带宽和内存越多。"),
                #[local_ref]
                max_connections_row -> adw::SpinRow {},
                #[local_ref]
                max_active_files_row -> adw::SpinRow {},
                #[local_ref]
                max_workers_row -> adw::SpinRow {},
            },

            add = &adw::PreferencesGroup {
                set_title: "分块",
                set_description: Some("大文件拆成多个分块并行下载。"),
                #[local_ref]
                chunk_threshold_row -> adw::SpinRow {},
                #[local_ref]
                chunk_size_row -> adw::SpinRow {},
            },

            add = &adw::PreferencesGroup {
                set_title: "重试",
                set_description: Some("服务器返回 Retry-After 时以响应头为准。"),
                #[local_ref]
                max_retries_row -> adw::SpinRow {},
                #[local_ref]
                retry_base_delay_row -> adw::SpinRow {},
                #[local_ref]
                retry_max_delay_row -> adw::SpinRow {},
            },

            add = &adw::PreferencesGroup {
                set_title: "安装",
                set_description: Some("安装器 processor 使用的 Java，不影响启动游戏。"),
                #[local_ref]
                java_expander -> adw::ExpanderRow {},
            },
        }
    }

    fn init(
        init: Self::Init,
        root_widget: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let DownloadPageInit { shell, cache } = init;
        let config = config_store::store().config();
        let download = config.read().download.clone();
        let syncing = Rc::new(Cell::new(false));

        // 页面常驻，但只有真正显示时才去执行 Java，避免启动时探测。
        root_widget.connect_map({
            let sender = sender.clone();
            move |_| {
                let _ = sender.input_sender().send(DownloadMsg::Shown);
            }
        });
        root_widget.connect_unmap({
            let sender = sender.clone();
            move |_| {
                let _ = sender.input_sender().send(DownloadMsg::Hidden);
            }
        });

        let mirror_row = adw::ComboRow::builder()
            .title("镜像策略")
            .subtitle("官方源与镜像源的使用顺序")
            .model(&gtk::StringList::new(&MIRROR_LABELS))
            .selected(mirror_index(download.mirror))
            .build();
        connect_combo(&mirror_row, &syncing, &sender, DownloadMsg::MirrorSelected);

        let skip_music_row = adw::SwitchRow::builder()
            .title("跳过音乐与音效")
            .subtitle("不下载资源文件里的 *.ogg，可节省流量")
            .active(download.skip_music)
            .build();
        connect_switch(
            &skip_music_row,
            &syncing,
            &sender,
            DownloadMsg::SkipMusicChanged,
        );

        let max_connections_row = spin_row(
            "最大连接数",
            "同时进行的 HTTP 请求数上限",
            download.max_connections as u64,
            1,
        );
        connect_spin(
            &max_connections_row,
            &syncing,
            &sender,
            DownloadMsg::MaxConnectionsChanged,
        );

        let max_active_files_row = spin_row(
            "同时活跃文件数",
            "同时下载的文件数上限",
            download.max_active_files as u64,
            1,
        );
        connect_spin(
            &max_active_files_row,
            &syncing,
            &sender,
            DownloadMsg::MaxActiveFilesChanged,
        );

        let max_workers_row = spin_row(
            "工作线程数",
            "预检与校验等任务的并发上限",
            download.max_workers as u64,
            1,
        );
        connect_spin(
            &max_workers_row,
            &syncing,
            &sender,
            DownloadMsg::MaxWorkersChanged,
        );

        let chunk_threshold_row = spin_row(
            "分块阈值",
            "超过该大小的文件分块下载（MiB）",
            download.chunk_threshold_mib,
            1,
        );
        connect_spin(
            &chunk_threshold_row,
            &syncing,
            &sender,
            DownloadMsg::ChunkThresholdChanged,
        );

        let chunk_size_row = spin_row(
            "分块大小",
            "每个分块的大小（MiB）",
            download.chunk_size_mib,
            1,
        );
        connect_spin(
            &chunk_size_row,
            &syncing,
            &sender,
            DownloadMsg::ChunkSizeChanged,
        );

        let max_retries_row = spin_row(
            "最大重试次数",
            "单个文件失败后最多重试几次",
            u64::from(download.max_retries),
            0,
        );
        connect_spin(
            &max_retries_row,
            &syncing,
            &sender,
            DownloadMsg::MaxRetriesChanged,
        );

        let retry_base_delay_row = spin_row(
            "首次重试延迟",
            "第一次重试前的等待时间（毫秒），之后每次翻倍",
            download.retry_base_delay_ms,
            0,
        );
        connect_spin(
            &retry_base_delay_row,
            &syncing,
            &sender,
            DownloadMsg::RetryBaseDelayChanged,
        );

        let retry_max_delay_row = spin_row(
            "最大重试延迟",
            "指数退避的等待上限（毫秒）",
            download.retry_max_delay_ms,
            0,
        );
        connect_spin(
            &retry_max_delay_row,
            &syncing,
            &sender,
            DownloadMsg::RetryMaxDelayChanged,
        );

        let java_expander = adw::ExpanderRow::builder()
            .title("安装器 Java")
            .show_enable_switch(false)
            .build();
        let mut java_list =
            JavaChoices::new(
                &java_expander,
                cache,
                sender.input_sender(),
                |out| match out {
                    ChoiceRowOut::Selected(index) => DownloadMsg::JavaSelected(index),
                },
            );
        {
            let config = config.read();
            java_list.reload(&config, "");
        }

        let model = Self {
            config,
            shell,
            page_toast: PageToast::new(&root_widget),
            syncing,
            download,
            java_list,
            dirty: false,
            flush: None,
            mirror_row: mirror_row.clone(),
            skip_music_row: skip_music_row.clone(),
            max_connections_row: max_connections_row.clone(),
            max_active_files_row: max_active_files_row.clone(),
            max_workers_row: max_workers_row.clone(),
            chunk_threshold_row: chunk_threshold_row.clone(),
            chunk_size_row: chunk_size_row.clone(),
            max_retries_row: max_retries_row.clone(),
            retry_base_delay_row: retry_base_delay_row.clone(),
            retry_max_delay_row: retry_max_delay_row.clone(),
            java_expander: java_expander.clone(),
        };

        let widgets = view_output!();
        model.sync_ui();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>) {
        match message {
            DownloadMsg::Shown => {
                self.reload();
                self.java_list.set_visible(true);
                self.probe_missing(&sender);
            }
            DownloadMsg::Hidden => self.java_list.set_visible(false),
            DownloadMsg::ConfigChanged => {
                // 正在编辑时不覆盖用户还没写回的值；写回后根组件会再发一次。
                if !self.dirty {
                    self.reload();
                    self.probe_missing(&sender);
                }
            }
            DownloadMsg::MirrorSelected(index) => {
                self.download.mirror = mirror_from_index(index);
                self.mark_dirty(&sender);
            }
            DownloadMsg::SkipMusicChanged(active) => {
                self.download.skip_music = active;
                self.mark_dirty(&sender);
            }
            DownloadMsg::MaxConnectionsChanged(value) => {
                self.download.max_connections = value as usize;
                self.mark_dirty(&sender);
            }
            DownloadMsg::MaxActiveFilesChanged(value) => {
                self.download.max_active_files = value as usize;
                self.mark_dirty(&sender);
            }
            DownloadMsg::MaxWorkersChanged(value) => {
                self.download.max_workers = value as usize;
                self.mark_dirty(&sender);
            }
            DownloadMsg::ChunkThresholdChanged(value) => {
                self.download.chunk_threshold_mib = u64::from(value);
                self.mark_dirty(&sender);
            }
            DownloadMsg::ChunkSizeChanged(value) => {
                self.download.chunk_size_mib = u64::from(value);
                self.mark_dirty(&sender);
            }
            DownloadMsg::MaxRetriesChanged(value) => {
                self.download.max_retries = value;
                self.mark_dirty(&sender);
            }
            DownloadMsg::RetryBaseDelayChanged(value) => {
                self.download.retry_base_delay_ms = u64::from(value);
                self.mark_dirty(&sender);
            }
            DownloadMsg::RetryMaxDelayChanged(value) => {
                self.download.retry_max_delay_ms = u64::from(value);
                self.mark_dirty(&sender);
            }
            DownloadMsg::JavaSelected(index) => {
                if index as usize > self.java_list.javas().len() {
                    return;
                }
                let current = self.java_list.index_for_choice(&self.download.java);
                if index != current {
                    self.download.java = self.java_list.choice_for_index(index);
                    self.mark_dirty(&sender);
                }
                self.sync_ui();
            }
            DownloadMsg::Probed { path, result } => {
                self.java_list.apply_probe(&path, result);
                self.sync_ui();
            }
            DownloadMsg::Flush => {
                self.flush = None;
                self.flush_pending();
            }
        }
    }
}

impl DownloadPage {
    /// 从共享配置重新读取下载设置和 Java 表；有未写回的改动时保留本地设置。
    fn reload(&mut self) {
        {
            let config = self.config.read();
            if !self.dirty {
                self.download = config.download.clone();
            }
            self.java_list.reload(&config, "");
        }
        self.sync_ui();
    }

    /// 探测还没缓存的安装器 Java；页面不可见时什么也不做。
    fn probe_missing(&mut self, sender: &ComponentSender<Self>) {
        self.java_list
            .probe_missing(sender.input_sender(), |path, result| DownloadMsg::Probed {
                path,
                result,
            });
    }

    /// 把当前值写回控件；信号回调被 `syncing` 屏蔽。
    fn sync_ui(&self) {
        self.syncing.set(true);
        self.mirror_row
            .set_selected(mirror_index(self.download.mirror));
        self.skip_music_row.set_active(self.download.skip_music);
        set_spin(
            &self.max_connections_row,
            self.download.max_connections as u64,
        );
        set_spin(
            &self.max_active_files_row,
            self.download.max_active_files as u64,
        );
        set_spin(&self.max_workers_row, self.download.max_workers as u64);
        set_spin(&self.chunk_threshold_row, self.download.chunk_threshold_mib);
        set_spin(&self.chunk_size_row, self.download.chunk_size_mib);
        set_spin(&self.max_retries_row, u64::from(self.download.max_retries));
        set_spin(
            &self.retry_base_delay_row,
            self.download.retry_base_delay_ms,
        );
        set_spin(&self.retry_max_delay_row, self.download.retry_max_delay_ms);

        let index = self.java_list.index_for_choice(&self.download.java);
        self.java_list.sync(index, true);
        self.java_expander
            .set_subtitle(&self.java_list.label_of(&self.download.java));
        self.syncing.set(false);
    }

    /// 记下改动并安排一次写回。
    fn mark_dirty(&mut self, sender: &ComponentSender<Self>) {
        self.dirty = true;
        self.cancel_flush();
        let sender = sender.clone();
        self.flush = Some(glib::timeout_add_local_once(FLUSH_DELAY, move || {
            let _ = sender.input_sender().send(DownloadMsg::Flush);
        }));
    }

    fn cancel_flush(&mut self) {
        if let Some(source) = self.flush.take() {
            source.remove();
        }
    }

    /// 把整段下载配置写回共享配置；保存失败时在页面上提示。
    fn flush_pending(&mut self) {
        self.cancel_flush();
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let download = self.download.clone();
        let shell = self.shell.clone();
        let page_toast = self.page_toast.clone();
        config_store::store().update(
            move |config| config.download = download,
            move |err| shell.page_toast(&page_toast, &format!("保存配置失败：{err:#}")),
        );
    }
}

fn spin_row(title: &str, subtitle: &str, value: u64, min: u32) -> adw::SpinRow {
    let value = to_u32(value);
    let adjustment = gtk::Adjustment::new(
        f64::from(value),
        f64::from(min),
        f64::from(u32::MAX),
        1.0,
        10.0,
        0.0,
    );
    adw::SpinRow::builder()
        .title(title)
        .subtitle(subtitle)
        .adjustment(&adjustment)
        .climb_rate(1.0)
        .digits(0)
        .numeric(true)
        .value(f64::from(value))
        .build()
}

/// 配置里的计数 / 大小 / 毫秒数 → 控件能表示的 `u32`。
fn to_u32(value: u64) -> u32 {
    value.min(u64::from(u32::MAX)) as u32
}

fn set_spin(row: &adw::SpinRow, value: u64) {
    row.set_value(f64::from(to_u32(value)));
}

fn connect_spin(
    row: &adw::SpinRow,
    syncing: &Rc<Cell<bool>>,
    sender: &ComponentSender<DownloadPage>,
    message: fn(u32) -> DownloadMsg,
) {
    let syncing = Rc::clone(syncing);
    let sender = sender.clone();
    row.connect_value_notify(move |row| {
        if syncing.get() {
            return;
        }
        let _ = sender.input_sender().send(message(row.value() as u32));
    });
}

fn connect_combo(
    row: &adw::ComboRow,
    syncing: &Rc<Cell<bool>>,
    sender: &ComponentSender<DownloadPage>,
    message: fn(u32) -> DownloadMsg,
) {
    let syncing = Rc::clone(syncing);
    let sender = sender.clone();
    row.connect_selected_notify(move |row| {
        if syncing.get() {
            return;
        }
        let _ = sender.input_sender().send(message(row.selected()));
    });
}

fn connect_switch(
    row: &adw::SwitchRow,
    syncing: &Rc<Cell<bool>>,
    sender: &ComponentSender<DownloadPage>,
    message: fn(bool) -> DownloadMsg,
) {
    let syncing = Rc::clone(syncing);
    let sender = sender.clone();
    row.connect_active_notify(move |row| {
        if syncing.get() {
            return;
        }
        let _ = sender.input_sender().send(message(row.is_active()));
    });
}

fn mirror_index(policy: MirrorPolicy) -> u32 {
    MirrorPolicy::ALL
        .iter()
        .position(|candidate| *candidate == policy)
        .unwrap_or(0) as u32
}

fn mirror_from_index(index: u32) -> MirrorPolicy {
    MirrorPolicy::ALL
        .get(index as usize)
        .copied()
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirror_selection_follows_the_policy_order() {
        for (index, policy) in MirrorPolicy::ALL.into_iter().enumerate() {
            assert_eq!(mirror_index(policy), index as u32);
            assert_eq!(mirror_from_index(index as u32), policy);
        }
        // 配置里的非法下标回退到默认策略，而不是停在一个没有对应的选项上。
        assert_eq!(mirror_from_index(u32::MAX), MirrorPolicy::default());
    }

    #[test]
    fn values_too_large_for_the_spinner_are_capped() {
        assert_eq!(to_u32(u64::from(u32::MAX)), u32::MAX);
        assert_eq!(to_u32(u64::from(u32::MAX) + 1), u32::MAX);
    }
}
