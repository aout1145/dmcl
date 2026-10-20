//! 加载器下拉的静态定义、运行时槽位与初始化。

use std::sync::Arc;

use relm4::prelude::*;

use dmcl_install::LoaderKind;
use dmcl_mcmeta::source::DownloadSource;

use crate::gui::gtk4::pages::download::common::version_select::{
    VersionItem, VersionSelect, VersionSelectInit,
};

use super::versions::{fabric_items, forge_items, neoforge_items};

/// 一个加载器下拉的静态定义。
pub(super) struct LoaderDef {
    pub(super) kind: LoaderKind,
    /// 冲突组：同组里只能选一个；选中一个后其它同组行会被禁用（`None` = 不参与冲突）。
    ///
    /// 现在三个加载器同属 `"loader"`；将来若要允许叠加（比如双加载器整合包），改这里
    /// 的组名即可，页面逻辑不用动。
    pub(super) mutex_group: Option<&'static str>,
}

/// 加载器下拉表（也是页面上的显示顺序）。
pub(super) const LOADER_DEFS: &[LoaderDef] = &[
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

/// 一个加载器下拉 + 它当前的选择。
pub(super) struct LoaderSlot {
    pub(super) kind: LoaderKind,
    pub(super) mutex_group: Option<&'static str>,
    pub(super) select: Controller<VersionSelect>,
    pub(super) selection: Option<VersionItem>,
}

/// 加载器下拉的 Init（取数器捕获下载源，按上下文取列表）。
pub(super) fn loader_init(kind: LoaderKind, source: DownloadSource) -> VersionSelectInit {
    match kind {
        // Forge：列表新→旧；推荐版本来自 `branch == "recommended"`。
        // 类别 chips 留给后续（TODO）。
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
            show_search: false,
            allow_none: true,
            initially_expanded: false,
        },
        // Fabric：不区分版本类型；第一条「最新版本」，第一条 stable「推荐版本」。
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
            chips: Vec::new(),
            show_search: false,
            allow_none: true,
            initially_expanded: false,
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
            show_search: false,
            allow_none: true,
            initially_expanded: false,
        },
    }
}
