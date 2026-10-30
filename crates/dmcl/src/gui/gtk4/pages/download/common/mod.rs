//! download 页面的公共组件。
//!
//! 组件与交互细节见下载域的 `../README.md`。

pub(crate) mod instance_summary;
pub(crate) mod version_select;

/// 组件的公共样式：版本列表里的选中行高亮。
///
/// ListBox 会给每个工厂行套一层隐式的、可悬停的 `GtkListBoxRow`；行内又是
/// `GtkButton`。两层的 hover 背景叠在一起就会看到「外圈浅、内圈深」两圈强调色，
/// 所以这里把外层行的 hover / active 背景清掉，只保留按钮自己的一层。
pub(crate) const CSS: &str = "\
.dmcl-version-list > row.activatable:hover,
.dmcl-version-list > row.activatable:active,
.dmcl-version-list > row.activatable.has-open-popup { background: none; }
/* 展开区整块内容被 ExpanderRow 的嵌套 list 包成一行；清掉它的 hover，只让里面
   的搜索框、chips、版本行自己响应鼠标。 */
.dmcl-version-select list.nested > row.dmcl-version-panel-row:hover,
.dmcl-version-select list.nested > row.dmcl-version-panel-row:active { background: none; }
/* chips 用 caption 字号，但 `.pill` 的默认 padding 太大，收紧成紧凑药丸。 */
button.dmcl-version-chip { min-height: 0; padding: 3px 14px; }
/* 分类选择器里的可选项比默认 ActionRow 矮一点，避免列表占太高。 */
.dmcl-category-choice > box.header { min-height: 36px; }
.dmcl-category-choice > box.header > box.title { margin-top: 2px; margin-bottom: 2px; }
.dmcl-version-selected,
.dmcl-version-selected:hover,
.dmcl-version-selected:active { background: color-mix(in srgb, currentColor 12%, transparent); }";
