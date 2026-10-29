//! dmcl 应用层：配置 + 前端。
//!
//! 这一层是**唯一**知道配置文件存在的地方：它把 `dmcl.json` 读成 [`config::Config`]，
//! 再翻译成各业务 crate 需要的 `*Options`。前端有命令行（[`cli`]）与图形界面
//! （[`gui`]，GTK4，feature 门控）；业务 crate 一行都不用改。

pub mod build_info;
pub mod cli;
pub mod config;
pub mod gui;

pub use config::Config;
