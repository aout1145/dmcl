//! 模组加载器（Fabric / Forge / NeoForge）与 OptiFine 的安装。
//!
//! 每个加载器一个子模块，含它往安装图里追加节点的方式。加载器只下载自己的依赖并返回
//! `VersionJsonPatch`，不读写版本 JSON；所有 patch 由安装流程最后的「完成」步骤统一
//! 应用并写盘。Forge / NeoForge 拆成下载安装器、解析、准备依赖、跑 processor、生成
//! patch 几步。Forge 与 NeoForge 今天逻辑相近，但刻意**各写一份**，以便将来各自演进
//! 互不影响。OptiFine 是叠加在原版 / Forge 上的 tweaker，不是独立的加载器。

pub mod fabric;
pub mod forge;
pub mod neoforge;
pub mod optifine;

pub(crate) mod xforge;
