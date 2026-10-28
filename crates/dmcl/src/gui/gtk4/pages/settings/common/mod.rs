//! 首选项各分类共用的小组件。

pub(crate) mod drag_list;
pub(crate) mod java_cache;
pub(crate) mod java_display;

pub(crate) use drag_list::CSS;
pub(crate) use java_cache::JavaProbeCache;
pub(crate) use java_display::{details_arch_text, details_title, java_summary};
