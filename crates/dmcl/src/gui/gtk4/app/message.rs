//! 顶层消息。

use dmcl_mcmeta::layout::InstalledVersion;

/// 顶层组件收到的消息。
#[derive(Debug)]
pub(crate) enum AppMsg {
    /// 重新读取已安装的版本。
    ReloadInstances,
    /// 已安装版本列表读取完成（失败时是给用户看的一句话）。
    InstancesLoaded(Result<Vec<InstalledVersion>, String>),
    /// 启动某个实例。
    Launch(String),
    /// 打开某个实例的「管理」页。
    Manage(String),
    /// 在主界面弹一行提示。
    Toast(String),
}
