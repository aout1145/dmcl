//! 主页的消息。

use dmcl_mcmeta::layout::InstalledVersion;
use relm4::factory::DynamicIndex;

/// 主页组件收到的消息。
#[derive(Debug)]
pub(crate) enum HomeMsg {
    /// 重新读取已安装的版本。
    Reload,
    /// 版本列表读取完成。
    Loaded(Result<Vec<InstalledVersion>, String>),
    /// 选中某个磁贴。
    Select(DynamicIndex),
    /// 取消选中。
    Deselect,
    /// 双击启动某个磁贴。
    Launch(DynamicIndex),
    /// 按下操作栏的启动 / 取消 / 停止按钮。
    Toggle,
    /// 游戏进程已开始。
    GameStarted(String),
    /// 游戏进程已退出。`abnormal` 表示非正常退出（非 0 退出码 / 等不到状态）。
    GameExited { name: String, abnormal: bool },
    /// 启动失败（任务层已经会提示失败，这里只清理状态）。
    LaunchFailed(String),
    /// 打开选中实例的「管理」页。
    Manage,
}
