//! 版本选择器（download 页面公共组件）：置顶 + 搜索/类别筛选 + 动态取数。
//!
//! 组件不知道 Minecraft 的存在：列表取数器（[`FetchVersions`]）、筛选类别
//! （[`KindChip`]）都由页面在 Init 里给；「最新 / 推荐 / 稳定」这类置顶判定由取数
//! 适配层做好（条目带 [`Badge`] 就进置顶区），组件只负责展示与过滤。
//!
//! 形态：行 = `AdwExpanderRow`，标题 / 副标题显示当前状态，展开区里是可选搜索框、
//! 可选筛选 chips 与版本列表（加载器可以两者都不显示）。选中某个版本后立即收起；MC 行
//! 可由页面配置为首次可展开时自动展开。带 [`Badge`] 的条目全部进置顶区（可多条），徽标
//! 只在置顶行显示；下方完整列表不重复标签。类型列直接复用 chip 文案，没有 chip 就不显示。
//! 搜索框非空时置顶区整体隐藏，只保留过滤后的完整列表。
//!
//! 状态机：
//!
//! * `Idle` —— 还没给上下文，行不可展开；
//! * `Loading` —— 展开区里转 spinner；
//! * `Ready` —— 正常列表（置顶区 + 过滤后的完整列表）；
//! * `Empty` —— 取到了但列表为空（Forge 对不少版本没有构建）：行不可展开、副标题
//!   「无可用版本」；
//! * `Failed` —— 行尾显示刷新按钮，展开区只显示左对齐的错误信息。
//!
//! 动态取数带**代际计数**：上下文变化（换 MC 版本）时重新发起，过期响应直接丢弃，
//! 不会被慢响应覆盖。取到新列表后，原来选中的条目若不在其中会自动清掉并发
//! [`SelectOut::Changed(None)`]（页面据此把该加载器从装配态去掉）。

use std::sync::Arc;

use anyhow::Result;
use relm4::adw::prelude::*;
use relm4::gtk;
use relm4::prelude::*;

use dmcl_task::BoxFuture;

/// 一个可选项。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VersionItem {
    /// 传给安装器的版本 id（forge full_name / neoforge version / loader version）。
    pub(crate) id: String,
    /// 列表里显示的主文案，也用于自动命名（多数与 id 相同；Forge 是不带 mcversion 的
    /// `version` 字段，而 id 仍是安装用的 `full_name`）。
    pub(crate) label: String,
    /// 筛选用的类别（chips 的 kind 值），空串 = 无类别（不吃类别筛选但吃搜索）。
    pub(crate) kind: String,
    /// 置顶徽标（开放词汇：key 用于区分，label 是界面文案）。非空就进置顶区。
    pub(crate) badges: Vec<Badge>,
    /// 次要信息（发布日期 / build 号等），caption 字号，可空。
    pub(crate) detail: Option<String>,
}

/// 一条显示用的徽标（不给枚举：适配层想要什么徽标就给什么，避免把
///「最新 / 推荐 / 稳定」写成硬编码词汇表）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Badge {
    /// 样式 / 识别用的小写 key（"latest" / "recommended" / "stable" / 任意新值）。
    pub(crate) key: String,
    /// 界面文案（「最新正式版」「推荐版本」……）。
    pub(crate) label: String,
}

/// 类别筛选 chip。
#[derive(Debug, Clone)]
pub(crate) struct KindChip {
    /// 对应 [`VersionItem::kind`] 的取值（可把几个原始 kind 合并映射成一个）。
    pub(crate) kind: String,
    /// 显示文案。
    pub(crate) label: String,
    /// 初始是否参与筛选（由页面按版本类型给默认值，不再无脑全选）。
    pub(crate) default_selected: bool,
}

impl KindChip {
    pub(crate) fn new(kind: &str, label: &str, default_selected: bool) -> Self {
        Self {
            kind: kind.to_string(),
            label: label.to_string(),
            default_selected,
        }
    }
}

/// 取数器：按上下文（当前是 MC 版本 id；MC 版本清单的取数器忽略上下文）取可用版本。
/// `DownloadSource` 由页面在构造闭包时捕获并传入，组件不认识它。
pub(crate) type FetchVersions =
    Arc<dyn Fn(&str) -> BoxFuture<'_, Result<Vec<VersionItem>>> + Send + Sync>;

/// 组件 Init。
pub(crate) struct VersionSelectInit {
    /// 行标题，如「Forge 版本」。
    pub(crate) title: String,
    pub(crate) fetch: FetchVersions,
    /// 筛选 chips（空 = 没有类别筛选，也不显示版本类型列）；每个 chip 自带默认选中状态。
    pub(crate) chips: Vec<KindChip>,
    /// 是否显示搜索框（MC = true；加载器 = false）。
    pub(crate) show_search: bool,
    /// 是否允许未选择（加载器 = true，空选择即「不安装」；MC = false 必选）。
    pub(crate) allow_none: bool,
    /// 第一次变为可展开时是否自动展开（MC 行 = true；加载器行 = false）。
    pub(crate) initially_expanded: bool,
}

/// 组件输入。
#[derive(Debug)]
pub(crate) enum SelectIn {
    /// 动态取数的上下文（MC 版本 id）；`None` → 回到 `Idle` 并清空。
    /// 上下文变化会清掉旧选择并发 [`SelectOut::Changed(None)`]（旧选择已失效）。
    SetContext(Option<String>),
    /// 同组已有别的加载器被选中：`Some("Forge")` → 副标题显示「与 Forge 冲突」并禁用本行。
    SetConflict(Option<String>),
    /// 重新取数（Failed 状态的行尾刷新按钮）。
    Reload,
    /// 用户点了行上的 ×，清除选择（发输出）。
    Deselect,
    /// 搜索词变化。
    SearchChanged(String),
    /// 类别 chip 翻转。
    ToggleChip(usize),
    /// 列表里的某一行被点了（`pinned` = 置顶区还是完整列表；index 走工厂索引）。
    RowClicked { pinned: bool, index: DynamicIndex },
    /// 取数结果回来了（`relm4::spawn` 里发出）。
    Loaded {
        /// 发起时的代际；不是当前代就是过期响应，丢弃。
        gen: u64,
        result: Result<Vec<VersionItem>, String>,
    },
}

/// 组件输出。
#[derive(Debug)]
pub(crate) enum SelectOut {
    /// 选择变化；`None` = 不安装 / 旧选择失效。
    Changed(Option<VersionItem>),
}

/// 取数状态。
#[derive(Debug)]
enum LoadState {
    /// 还没给过上下文：行不可点。
    Idle,
    Loading,
    /// 列表（非空）。
    Ready(Vec<VersionItem>),
    /// 取到了但列表为空：该上下文下没有可用版本（Forge 常见）。
    Empty,
    Failed(String),
}

impl LoadState {
    /// 展开区里 `GtkStack` 当前应显示的页面。
    fn stack_page(&self) -> &'static str {
        match self {
            LoadState::Failed(_) => "failed",
            LoadState::Loading => "loading",
            LoadState::Idle | LoadState::Empty | LoadState::Ready(_) => "content",
        }
    }
}

/// 单个版本选择器。
pub(crate) struct VersionSelect {
    title: String,
    fetch: FetchVersions,
    chips: Vec<KindChip>,
    allow_none: bool,
    /// 同组已选加载器的名字；`Some` 时本行禁用并显示冲突提示。
    conflict: Option<String>,
    /// 首次可展开时自动展开；消费一次后置 false，避免用户手动收起又被状态同步展开。
    expand_on_first_show: bool,
    /// 当前上下文（== MC 版本 id；对 MC 版本本体的取数器没意义）。
    context: Option<String>,
    state: LoadState,
    selected: Option<VersionItem>,
    /// 取数代际：发起一次取数 +1，过期响应丢弃。
    gen: u64,
    /// 搜索词（小写比较在过滤函数里做）。
    search: String,
    /// 每个 chip 是否参与过滤（顺序对应 `chips`；初始值取自 `KindChip::default_selected`）。
    kinds_on: Vec<bool>,
    /// 置顶区的列表（所有 badge 非空的条目，可多条）。
    pinned_rows: FactoryVecDeque<VersionRow>,
    /// 完整列表（经搜索 / 类别过滤，保持数据源的新→旧顺序）。
    rows: FactoryVecDeque<VersionRow>,
    /// 承载标题、展开区的外层 ExpanderRow；用来按状态启用 / 收起展开。
    expander: adw::ExpanderRow,
    /// 展开区里的状态栈（content / loading / failed）。
    stack: gtk::Stack,
    /// 「全部版本」小节标题（有置顶项才显示）。
    all_label: gtk::Label,
    /// 失败页的错误文案。
    failed_label: gtk::Label,
}

impl VersionSelect {
    /// 行副标题（冲突提示 / 当前选择 / 状态文案）。
    fn display_text(&self) -> String {
        if let Some(conflict) = &self.conflict {
            return format!("与 {conflict} 冲突");
        }
        match &self.state {
            LoadState::Idle => "（先选择 Minecraft 版本）".to_string(),
            LoadState::Loading => "载入中…".to_string(),
            LoadState::Ready(_) => match &self.selected {
                Some(selected) => selected.label.clone(),
                None if self.allow_none => "不安装".to_string(),
                None => "请选择版本…".to_string(),
            },
            LoadState::Empty => "无可用版本".to_string(),
            LoadState::Failed(_) => "获取失败".to_string(),
        }
    }

    /// 这一行是否允许展开（Idle / Empty / 冲突不可；Loading / Failed 允许展开看状态与错误）。
    fn can_expand(&self) -> bool {
        self.conflict.is_none() && !matches!(self.state, LoadState::Idle | LoadState::Empty)
    }

    /// 置顶区是否有内容（决定「全部版本」小节标题是否显示）。
    /// 搜索时置顶区隐藏，避免不匹配的置顶版本混在搜索结果里。
    fn pinned_visible(&self) -> bool {
        if !self.search.is_empty() {
            return false;
        }
        match &self.state {
            LoadState::Ready(items) => items.iter().any(|item| !item.badges.is_empty()),
            _ => false,
        }
    }

    /// 选中某个条目（发输出），并立即收起展开区。
    fn select(&mut self, item: VersionItem, sender: &ComponentSender<VersionSelect>) {
        self.selected = Some(item);
        self.refresh_selection_flags();
        self.expander.set_expanded(false);
        let _ = sender.output(SelectOut::Changed(self.selected.clone()));
    }

    /// 把两个列表工厂里各行的选中标记刷新成当前的 `selected`。
    fn refresh_selection_flags(&mut self) {
        let selected = self.selected.as_ref().map(|item| item.id.clone());
        for factory in [&mut self.pinned_rows, &mut self.rows] {
            let mut guard = factory.guard();
            // `iter_mut()` 会把每一行都标成「变了」，900 行的 MC 列表一次选中就要重绘整张表。
            // 先只读地找出真正需要变的行，再逐行取可变引用，通常只有旧的/新的两行。
            let updates: Vec<(usize, bool)> = guard
                .iter()
                .enumerate()
                .filter_map(|(index, row)| {
                    let want = selected.as_ref().is_some_and(|id| *id == row.item.id);
                    (row.selected != want).then_some((index, want))
                })
                .collect();
            for (index, want) in updates {
                guard[index].selected = want;
            }
        }
    }

    /// 按当前状态重建两个列表工厂的内容（顺序保持：数据源的新→旧）。
    fn refresh_rows(&mut self) {
        let items = match &self.state {
            LoadState::Ready(items) => items.clone(),
            _ => Vec::new(),
        };

        // 类型列直接复用筛选 chip 的文案：页面给 `kind` + 文案，组件不硬编码版本类型。
        let kind_label = |item: &VersionItem| {
            self.chips
                .iter()
                .find(|chip| chip.kind == item.kind)
                .map(|chip| chip.label.clone())
        };
        let pinned_items: Vec<VersionRowInit> = if self.search.is_empty() {
            items
                .iter()
                .filter(|item| !item.badges.is_empty())
                .map(|item| VersionRowInit {
                    item: item.clone(),
                    show_badges: true,
                    kind_label: kind_label(item),
                })
                .collect()
        } else {
            Vec::new()
        };
        let row_items: Vec<VersionRowInit> =
            filter_items(&items, &self.search, &self.kinds_on, &self.chips)
                .into_iter()
                .map(|item| VersionRowInit {
                    item: item.clone(),
                    show_badges: false,
                    kind_label: kind_label(item),
                })
                .collect();

        {
            let mut pinned = self.pinned_rows.guard();
            pinned.clear();
            for init in pinned_items {
                pinned.push_back(init);
            }
        }
        {
            let mut rows = self.rows.guard();
            rows.clear();
            for init in row_items {
                rows.push_back(init);
            }
        }
        self.refresh_selection_flags();
    }

    /// 同步展开区状态：`GtkStack` 页面、「全部版本」标题、行是否可展开。
    ///
    /// 展开区内容是命令式组装的，relm4 的 `#[watch]` 够不到，所以在每次状态变化
    /// 后显式同步（widget 字段在模型里，没有旁路状态）。
    fn sync_view(&mut self) {
        self.stack.set_visible_child_name(self.state.stack_page());
        let pinned = self.pinned_visible();
        self.pinned_rows.widget().set_visible(pinned);
        self.all_label.set_visible(pinned);
        if let LoadState::Failed(err) = &self.state {
            self.failed_label
                .set_text(&format!("获取版本列表失败：{err}"));
        }

        let can_expand = self.can_expand();
        let was_enabled = self.expander.enables_expansion();
        if was_enabled != can_expand {
            self.expander.set_enable_expansion(can_expand);
            if can_expand {
                // libadwaita 在 enable-expansion 从 false 变 true 时会顺手展开；
                // 除了配置了首次自动展开的行（MC），其余保持收起，等用户点标题栏。
                // 自动展开只消费一次，之后用户手动收起不会被后续状态同步重新展开。
                self.expander.set_expanded(self.expand_on_first_show);
                self.expand_on_first_show = false;
            }
        }
        if !can_expand {
            // Empty / Idle 时不能让用户停在一个空展开区里。
            self.expander.set_expanded(false);
        }
    }

    /// 发起一次取数（要求 `context` 已设好）。
    fn start_fetch(&mut self, sender: ComponentSender<VersionSelect>) {
        let Some(context) = self.context.clone() else {
            return;
        };
        self.gen += 1;
        let gen = self.gen;
        self.state = LoadState::Loading;
        self.refresh_rows();
        self.sync_view();

        let fetch = self.fetch.clone();
        let sender = sender.clone();
        relm4::spawn(async move {
            let result = fetch(&context).await.map_err(|err| format!("{err:#}"));
            // 页面可能已经关掉（例如点安装后立即关页）：组件通道关了只说明没人要结果，
            // 用 `input_sender().send` 静默丢弃，别让后台取数把进程 panic 掉。
            let _ = sender.input_sender().send(SelectIn::Loaded { gen, result });
        });
    }
}

/// 过滤（搜索 + 类别）：搜索对 `id` / `label` 做大小写不敏感子串；类别全关 = 不过滤。
/// Order：保持数据源按的原始顺序（列表本来就是新→旧排好）。
fn matches_filter(item: &VersionItem, search: &str, kinds: &[String]) -> bool {
    if !search.is_empty() {
        let needle = search.to_lowercase();
        if !item.id.to_lowercase().contains(&needle) && !item.label.to_lowercase().contains(&needle)
        {
            return false;
        }
    }
    kinds.is_empty() || kinds.contains(&item.kind)
}

/// 筛选一批条目（纯函数、可单测），输出按原有顺序保持引用。
fn filter_items<'a>(
    items: &'a [VersionItem],
    search: &str,
    kinds_on: &[bool],
    chips: &[KindChip],
) -> Vec<&'a VersionItem> {
    let kinds: Vec<String> = chips
        .iter()
        .zip(kinds_on)
        .filter(|(_, on)| **on)
        .map(|(chip, _)| chip.kind.clone())
        .collect();
    items
        .iter()
        .filter(|item| matches_filter(item, search, &kinds))
        .collect()
}

/// 一行版本的工厂初始化参数。
struct VersionRowInit {
    item: VersionItem,
    /// 是否在文案后显示「最新 / 推荐」等徽标；只有置顶区为 true。
    show_badges: bool,
    /// 右侧的类型列（如「正式版」）；由 `VersionItem::kind` 对应的 chip 文案得出。
    kind_label: Option<String>,
}

/// 一行版本（工厂组件）：主文案 + 徽标文字 + 次要信息；点击上报给 [`VersionSelect`]。
#[derive(Debug)]
struct VersionRow {
    item: VersionItem,
    /// 选中标记（由 [`VersionSelect::refresh_selection_flags`] 维护）。
    selected: bool,
    /// 是否显示徽标文字；下方完整列表不显示，避免重复标签。
    show_badges: bool,
    /// 右侧的类型列（如「正式版」）；无对应 chip 时为 `None`。
    kind_label: Option<String>,
}

impl VersionRow {
    fn css(&self) -> Vec<&'static str> {
        let mut classes = vec!["flat"];
        if self.selected {
            classes.push("dmcl-version-selected");
        }
        classes
    }

    /// 主文案：只有置顶区才把 badge 附在后面（`1.21.10（最新）`）；下方列表不显示标签。
    fn main_text(&self) -> String {
        if !self.show_badges || self.item.badges.is_empty() {
            self.item.label.clone()
        } else {
            let badges: Vec<&str> = self
                .item
                .badges
                .iter()
                .map(|badge| badge.label.as_str())
                .collect();
            format!("{}（{}）", self.item.label, badges.join("、"))
        }
    }
}

#[derive(Debug)]
enum RowOut {
    Clicked(DynamicIndex),
}

#[relm4::factory(pub(crate))]
impl FactoryComponent for VersionRow {
    type Init = VersionRowInit;
    type Input = ();
    type Output = RowOut;
    type CommandOutput = ();
    type ParentWidget = gtk::ListBox;
    type Index = DynamicIndex;

    view! {
        root = gtk::Button {
            set_has_frame: false,
            #[watch]
            set_css_classes: &self.css(),
            connect_clicked[sender, index] => move |_| {
                let _ = sender.output(RowOut::Clicked(index.clone()));
            },

            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 4,

                gtk::Label {
                    set_label: &self.main_text(),
                    set_xalign: 0.0,
                    set_hexpand: true,
                },

                gtk::Label {
                    set_visible: self.kind_label.is_some(),
                    set_label: self.kind_label.as_deref().unwrap_or(""),
                    add_css_class: "caption",
                },

                gtk::Label {
                    #[watch]
                    set_visible: self.item.detail.is_some(),
                    set_label: self.item.detail.as_deref().unwrap_or(""),
                    add_css_class: "dim-label",
                    add_css_class: "caption",
                },
            },
        }
    }

    fn init_model(init: Self::Init, _index: &DynamicIndex, _sender: FactorySender<Self>) -> Self {
        let VersionRowInit {
            item,
            show_badges,
            kind_label,
        } = init;
        Self {
            item,
            selected: false,
            show_badges,
            kind_label,
        }
    }
}

#[relm4::component(pub(crate))]
impl SimpleComponent for VersionSelect {
    type Init = VersionSelectInit;
    type Input = SelectIn;
    type Output = SelectOut;

    view! {
        adw::ExpanderRow {
            add_css_class: "dmcl-version-select",
            set_title: &model.title,
            #[watch]
            set_sensitive: model.conflict.is_none(),
            #[watch]
            set_subtitle: &model.display_text(),

            // 已选时出现在展开箭头左侧的 ×（只有可"不安装"的加载器行才有）。
            // Failed 时让位给刷新按钮，同一位置只显示一个。
            add_suffix = &gtk::Button {
                set_icon_name: "window-close-symbolic",
                add_css_class: "flat",
                set_valign: gtk::Align::Center,
                set_tooltip_text: Some("取消选择（不安装）"),
                #[watch]
                set_visible: model.allow_none
                    && model.selected.is_some()
                    && !matches!(model.state, LoadState::Failed(_)),
                connect_clicked[sender] => move |_| sender.input(SelectIn::Deselect),
            },

            // Failed 时在 × 的位置显示刷新；展开区只保留错误信息。
            add_suffix = &gtk::Button {
                set_icon_name: "view-refresh-symbolic",
                add_css_class: "flat",
                set_valign: gtk::Align::Center,
                set_tooltip_text: Some("刷新版本列表"),
                #[watch]
                set_visible: matches!(model.state, LoadState::Failed(_)),
                connect_clicked[sender] => move |_| sender.input(SelectIn::Reload),
            },
        }
    }

    fn init(
        init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        use relm4::gtk::prelude::*;

        let VersionSelectInit {
            title,
            fetch,
            chips,
            show_search,
            allow_none,
            initially_expanded,
        } = init;

        let pinned_rows = FactoryVecDeque::builder()
            .launch(gtk::ListBox::default())
            .forward(sender.input_sender(), |out| match out {
                RowOut::Clicked(index) => SelectIn::RowClicked {
                    pinned: true,
                    index,
                },
            });
        let rows = FactoryVecDeque::builder()
            .launch(gtk::ListBox::default())
            .forward(sender.input_sender(), |out| match out {
                RowOut::Clicked(index) => SelectIn::RowClicked {
                    pinned: false,
                    index,
                },
            });

        let pinned_list: &gtk::ListBox = pinned_rows.widget();
        for list in [pinned_list, rows.widget()] {
            list.set_selection_mode(gtk::SelectionMode::None);
            // 不用 `navigation-sidebar`：它会给 ListBox 的隐式行再加一层 hover 背景，
            // 和行内 Button 自己的 hover 叠成内外两圈强调色（见 common::CSS）。
            list.add_css_class("dmcl-version-list");
        }

        // ---------- 展开区内容（静态组装；状态相关的可见性由 `sync_view` 跟进） ----------
        let stack = gtk::Stack::new();
        // 高度跟随当前页：Failed 只占错误文案的高度，不被列表页的 220 最小高度撑开。
        stack.set_vhomogeneous(false);
        let all_label = gtk::Label::new(Some("全部版本"));
        let failed_label = gtk::Label::new(None);
        failed_label.set_wrap(true);

        // 搜索框（加载器不显示）。
        let search = show_search.then(|| {
            let search = gtk::SearchEntry::new();
            search.set_placeholder_text(Some("搜索版本…"));
            let sender = sender.clone();
            search.connect_search_changed(move |entry| {
                sender.input(SelectIn::SearchChanged(entry.text().into()));
            });
            search
        });

        // 筛选 chips（Init 静态内容）：先按页面给的默认值 `set_active` 再连信号，
        // 免得初始化本身就发一次 ToggleChip。
        let chips_box = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        for (index, chip) in chips.iter().enumerate() {
            let button = gtk::ToggleButton::with_label(&chip.label);
            button.add_css_class("pill");
            button.add_css_class("caption");
            button.add_css_class("dmcl-version-chip");
            button.set_active(chip.default_selected);
            {
                let sender = sender.clone();
                button.connect_toggled(move |_| sender.input(SelectIn::ToggleChip(index)));
            }
            chips_box.append(&button);
        }
        chips_box.set_visible(!chips.is_empty());

        // content 页：置顶区 + 完整列表。
        {
            let content = gtk::Box::new(gtk::Orientation::Vertical, 4);
            all_label.set_halign(gtk::Align::Start);
            all_label.add_css_class("heading");
            all_label.set_margin_top(4);
            content.append(pinned_list);
            content.append(&all_label);
            content.append(rows.widget());

            let scrolled = gtk::ScrolledWindow::new();
            scrolled.set_hscrollbar_policy(gtk::PolicyType::Never);
            // 展开区随内容变长，最多到 480；再长的列表在内部滚动。
            scrolled.set_min_content_height(220);
            scrolled.set_max_content_height(480);
            scrolled.set_propagate_natural_height(true);
            scrolled.set_child(Some(&content));
            stack.add_named(&scrolled, Some("content"));
        }

        // loading 页：spinner。
        {
            let loading = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            loading.set_valign(gtk::Align::Center);
            loading.set_halign(gtk::Align::Center);
            let spinner = gtk::Spinner::new();
            spinner.set_spinning(true);
            loading.append(&spinner);
            loading.append(&gtk::Label::new(Some("载入版本列表…")));
            stack.add_named(&loading, Some("loading"));
        }

        // failed 页：只显示错误信息，靠左上，不居中；刷新按钮在行尾。
        {
            failed_label.add_css_class("error");
            failed_label.set_xalign(0.0);
            failed_label.set_halign(gtk::Align::Fill);
            failed_label.set_valign(gtk::Align::Start);
            stack.add_named(&failed_label, Some("failed"));
        }

        {
            let content = gtk::Box::new(gtk::Orientation::Vertical, 6);
            content.set_margin_all(12);
            if let Some(search) = &search {
                content.append(search);
            }
            content.append(&chips_box);
            content.append(&stack);
            stack.set_visible_child_name("content");
            // `AdwExpanderRow` 的展开区是它内部的嵌套列表；整块内容作为一个子行。
            root.add_row(&content);
            // 嵌套列表会把这整块内容包进一个可悬停的 GtkListBoxRow；鼠标一进来整块
            // 展开区就变深。这里关掉该行的激活/悬停，只让里面的搜索框、chips、版本行
            // 自己响应鼠标。
            if let Some(row) = content
                .parent()
                .and_then(|widget| widget.downcast::<gtk::ListBoxRow>().ok())
            {
                row.set_activatable(false);
                row.set_selectable(false);
                row.add_css_class("dmcl-version-panel-row");
            }
        }

        let kinds_on = chips.iter().map(|chip| chip.default_selected).collect();
        let mut model = VersionSelect {
            title,
            fetch,
            chips,
            allow_none,
            conflict: None,
            expand_on_first_show: initially_expanded,
            context: None,
            state: LoadState::Idle,
            selected: None,
            gen: 0,
            search: String::new(),
            kinds_on,
            pinned_rows,
            rows,
            expander: root.clone(),
            stack,
            all_label,
            failed_label,
        };

        let widgets = view_output!();
        model.sync_view();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>) {
        match message {
            SelectIn::SetContext(context) => {
                // 同上下文且已经取到结果：不重取。
                if context == self.context
                    && matches!(self.state, LoadState::Ready(_) | LoadState::Empty)
                {
                    return;
                }
                self.context = context.clone();
                // 换上下文后，旧请求的响应一律作废（Idle 分支也要作废）。
                self.gen += 1;
                if self.allow_none && self.selected.is_some() {
                    // 换上下文 = 旧选择失效（比如 MC 版本换了，Forge 选的构建没了）。
                    self.selected = None;
                    let _ = sender.output(SelectOut::Changed(None));
                }
                match context {
                    Some(_) => self.start_fetch(sender.clone()),
                    None => {
                        self.state = LoadState::Idle;
                        self.refresh_rows();
                        self.sync_view();
                    }
                }
            }
            SelectIn::SetConflict(conflict) => {
                self.conflict = conflict;
                self.sync_view();
            }
            SelectIn::Reload => self.start_fetch(sender.clone()),
            SelectIn::Deselect => {
                if self.selected.is_some() {
                    self.selected = None;
                    self.refresh_selection_flags();
                    let _ = sender.output(SelectOut::Changed(None));
                }
            }
            SelectIn::SearchChanged(text) => {
                self.search = text;
                self.refresh_rows();
                self.sync_view();
            }
            SelectIn::ToggleChip(index) => {
                if let Some(state) = self.kinds_on.get_mut(index) {
                    *state = !*state;
                }
                self.refresh_rows();
            }
            SelectIn::RowClicked { pinned, index } => {
                let item = if pinned {
                    self.pinned_rows
                        .get(index.current_index())
                        .map(|row| row.item.clone())
                } else {
                    self.rows
                        .get(index.current_index())
                        .map(|row| row.item.clone())
                };
                if let Some(item) = item {
                    self.select(item, &sender);
                }
            }
            SelectIn::Loaded { gen, result } if gen == self.gen => {
                match result {
                    Ok(items) if !items.is_empty() => {
                        // 旧选择不在新列表里：清掉并输出（页面据此调整装配态）。
                        if self.selected.as_ref().is_some_and(|selected| {
                            !items.iter().any(|item| item.id == selected.id)
                        }) {
                            self.selected = None;
                            let _ = sender.output(SelectOut::Changed(None));
                        }
                        self.state = LoadState::Ready(items);
                        self.refresh_rows();
                        self.sync_view();
                    }
                    Ok(_) => {
                        // 列表为空：这个上下文下没有可用版本。
                        if self.selected.is_some() {
                            self.selected = None;
                            let _ = sender.output(SelectOut::Changed(None));
                        }
                        self.state = LoadState::Empty;
                        self.refresh_rows();
                        self.sync_view();
                    }
                    Err(err) => {
                        self.state = LoadState::Failed(err);
                        self.refresh_rows();
                        self.sync_view();
                    }
                }
            }
            // 过期响应（gen != self.gen），丢弃。
            SelectIn::Loaded { .. } => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, kind: &str) -> VersionItem {
        VersionItem {
            id: id.to_string(),
            label: id.to_string(),
            kind: kind.to_string(),
            badges: Vec::new(),
            detail: None,
        }
    }

    fn chips() -> Vec<KindChip> {
        vec![
            KindChip::new("release", "正式版", true),
            KindChip::new("snapshot", "快照版", false),
        ]
    }

    #[test]
    fn filtering_keeps_source_order_and_matches_case_insensitively() {
        let items = vec![
            item("1.21.10", "release"),
            item("24w00a", "snapshot"),
            item("1.0", "old"),
        ];

        // 类别全关 = 不按类别筛，顺序保持新→旧。
        let all = filter_items(&items, "", &[false, false], &chips());
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].id, "1.21.10");

        // 两个 chip 都亮着 = 只留清单里列出的类别（old 不在 chips 里）。
        assert_eq!(filter_items(&items, "", &[true, true], &chips()).len(), 2);

        // 搜索大小写不敏感，命中 id 或 label。
        assert_eq!(
            filter_items(&items, "24W", &[true, true], &chips()).len(),
            1
        );
        assert!(filter_items(&items, "missing", &[true, true], &chips()).is_empty());
    }

    #[test]
    fn kinds_toggle_independently() {
        let items = vec![item("1.21.10", "release"), item("24w00a", "snapshot")];

        // 只留快照。
        let snapshots = filter_items(&items, "", &[false, true], &chips());
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].id, "24w00a");

        // 只留正式版。
        let releases = filter_items(&items, "", &[true, false], &chips());
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].id, "1.21.10");

        // 搜索 + 类别联动。
        assert!(filter_items(&items, "1.21.10", &[false, true], &chips()).is_empty());
    }

    #[test]
    fn kinds_not_listed_in_chips_are_hidden_when_filtering() {
        let items = vec![item("a", "old")];
        assert!(filter_items(&items, "", &[true, true], &chips()).is_empty());
        assert_eq!(filter_items(&items, "", &[false, false], &chips()).len(), 1);
    }
}
