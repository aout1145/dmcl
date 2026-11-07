//! 模组加载器（Fabric / Forge / NeoForge）与 OptiFine 的安装。
//!
//! 每个加载器一个子模块，含它往安装图里追加节点的方式：Fabric / OptiFine 合并一次
//! 版本 JSON；Forge / NeoForge 拆成下载安装器、解析、准备依赖、跑 processor、合并
//! 版本 JSON 几步。Forge 与 NeoForge 今天逻辑相近，但刻意**各写一份**，以便将来各自
//! 演进互不影响。OptiFine 是叠加在原版 / Forge 上的 tweaker，不是独立的加载器。

pub mod fabric;
pub mod forge;
pub mod neoforge;
pub mod optifine;

pub(crate) mod common;
pub(crate) mod xforge;
