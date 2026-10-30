//! 主页的消息。

use dmcl_launch::GameMonitor;

use crate::config::LaunchConfig;

/// 一个已安装实例及其分类。
#[derive(Debug, Clone)]
pub(crate) struct InstanceEntry {
    pub(crate) name: String,
    pub(crate) category: Option<String>,
}

/// 主页组件收到的消息。
#[derive(Debug)]
pub(crate) enum HomeMsg {
    /// 重新读取已安装的版本。
    Reload,
    /// 版本列表读取完成。
    Loaded(Result<Vec<InstanceEntry>, String>),
    /// 选中某个磁贴。
    Select(String),
    /// 取消选中。
    Deselect,
    /// 双击启动某个磁贴。
    Launch(String),
    /// 按下操作栏的启动 / 取消 / 停止按钮。
    Toggle,
    /// 用户在实例配置警告框里点了「继续启动」。
    LaunchConfirmed { name: String, launch: LaunchConfig },
    /// 启动链成功拿到了游戏进程。
    Launched { name: String, monitor: GameMonitor },
    /// 启动链失败或被取消。
    LaunchFailed { name: String },
    /// 本页启动的进程已经退出。
    MonitorExited {
        name: String,
        stopped: bool,
        abnormal: bool,
    },
    /// 版本目录监听失败。
    WatchFailed(String),
    /// 打开选中实例的「管理」页。
    Manage,
    /// 打开「新建实例」方式选择框。
    CreateInstance,
    /// 共享配置变了：句柄指向新值，只需重画。
    ConfigChanged,
}
