//! Minecraft 元数据 crate。
//!
//! 这里集中三件事，它们都属于"描述游戏是什么"的范畴，而不涉及"把文件写到磁盘上"：
//!
//! 1. **强类型文档** —— 版本 JSON、版本清单、资源索引、安装器档案、模组加载器版本列表。
//!    每个结构体都带 `#[serde(flatten)] extra`，因此回写时不会丢掉未建模的字段。
//! 2. **下载源与 URL 模板** —— [`source::DownloadSource`] 把"从哪儿下"从"下什么"里抽出来：
//!    文档里所有 url 都以 `${MC_MAVEN}` 这样的**模板**形式存在，由 [`source::Root`] +
//!    [`source::Mirror`] 决定最终解析成官方地址还是镜像地址。
//! 3. **本地目录布局** —— `versions/<name>/<name>.json` 之类的路径约定（[`layout`]），以及
//!    读取版本 JSON（含 `inheritsFrom` 合并）。
//!
//! 模块按"域"划分：[`vanilla`]（原版）、[`fabric`]、[`forge`]、[`neoforge`]；
//! [`source`] / [`os`] / [`maven`] / [`merge`] / [`layout`] / [`json`] 是四个域共用的底座。
//!
//! 这个 crate **不读配置文件**：`root_path`、镜像、并发度等一律由调用者通过参数传入。

pub(crate) mod macros;

pub mod json;
pub mod layout;
pub mod maven;
pub mod merge;
pub mod os;
pub mod source;

pub mod fabric;
pub mod forge;
pub mod neoforge;
pub mod vanilla;
