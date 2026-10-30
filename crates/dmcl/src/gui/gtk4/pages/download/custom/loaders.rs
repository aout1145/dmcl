//! 加载器下拉的静态定义、运行时槽位与初始化。

use std::sync::Arc;

use relm4::prelude::*;

use dmcl_install::LoaderKind;
use dmcl_mcmeta::source::DownloadSource;

use crate::gui::gtk4::pages::download::common::version_select::{
    KindChip, VersionItem, VersionSelect, VersionSelectInit,
};

use super::versions::{fabric_items, forge_items, neoforge_items, optifine_items};

/// 一个加载器下拉的静态定义。
pub(super) struct LoaderDef {
    pub(super) kind: LoaderKind,
    /// 冲突组：只要两个选中的行共享任意一个组就冲突；空切片 = 不参与冲突。
    ///
    /// Forge / Fabric / NeoForge 同属 `"loader"`；Fabric / NeoForge / OptiFine 同属
    /// `"optifine"`。因此 Forge 可以与 OptiFine 叠加，而 Fabric / NeoForge 与两者都冲突。
    pub(super) mutex_groups: &'static [&'static str],
}

/// 加载器下拉表（也是页面上的显示顺序）。
pub(super) const LOADER_DEFS: &[LoaderDef] = &[
    LoaderDef {
        kind: LoaderKind::Forge,
        mutex_groups: &["loader"],
    },
    LoaderDef {
        kind: LoaderKind::Fabric,
        mutex_groups: &["loader", "optifine"],
    },
    LoaderDef {
        kind: LoaderKind::NeoForge,
        mutex_groups: &["loader", "optifine"],
    },
    LoaderDef {
        kind: LoaderKind::OptiFine,
        mutex_groups: &["optifine"],
    },
];

/// 一个加载器下拉 + 它当前的选择。
pub(super) struct LoaderSlot {
    pub(super) kind: LoaderKind,
    pub(super) mutex_groups: &'static [&'static str],
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
        // OptiFine：稳定版 / 预览版都可以选，最新两种各自置顶。
        LoaderKind::OptiFine => VersionSelectInit {
            title: format!("{} 版本", kind.display_name()),
            fetch: Arc::new(move |context: &str| {
                let source = source.clone();
                let context = context.to_string();
                Box::pin(async move {
                    let entries = source.fetch_optifine_versions(&context).await?;
                    Ok(optifine_items(entries))
                })
            }),
            chips: vec![
                KindChip::new("stable", "稳定版", true),
                KindChip::new("preview", "预览版", true),
            ],
            show_search: false,
            allow_none: true,
            initially_expanded: false,
        },
    }
}
