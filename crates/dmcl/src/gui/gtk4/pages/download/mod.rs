//! 下载相关页面。
//!
//! ```text
//! download/
//! ├── common/                # download 页面的公共组件
//! │   ├── instance_summary.rs  # 实例概要：图标 / 名称 / 分类 / 安装按钮
//! │   └── version_select.rs    # 版本下拉：置顶 + 搜索 / chips 筛选 + 动态取数
//! └── custom/                # 「自定义」页（新建实例）
//! ```

pub(crate) mod common;
pub(crate) mod custom;
