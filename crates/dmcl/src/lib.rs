//! dmcl 应用层：配置 + 前端。
//!
//! 这一层是**唯一**知道配置文件存在的地方：它把 `dmcl.json` 读成 [`config::Config`]，
//! 再翻译成各业务 crate 需要的 `*Options`。将来要加 GUI，只需在 `cli` 旁边再加一个
//! 前端模块，业务 crate 一行都不用改。

pub mod cli;
pub mod config;

pub use config::Config;
