# dmcl-account

账户：启动快照、登录源，以及离线 / 微软账户实现。

不依赖 workspace 内的其它业务 crate，也**不读全局配置**：凭据怎么保存由调用方实现
`RefreshTokenStore`，交互式登录怎么展示由调用方实现 `DeviceCodePresenter`。启动链的登录
节点、CLI 的 `login` 与图形界面的账户页都只是它的调用方。

## 模块布局

```text
account.rs    Account（登录快照）与 AccountSource（登录 / 刷新）
offline.rs    离线账户：由名字派生 uuid，token 为空
microsoft.rs  微软账户：设备代码登录、refresh token 刷新、Xbox / Minecraft 会话
```

## 设计意图

### 账户分两层

`Account` 是登录完成后用于拼命令行的只读快照，`AccountSource` 负责登录 / 刷新并产出快照。
账户层不依赖启动链或任务系统，也不认识 `dmcl-launch`，启动链只是它的调用方之一。

`AccountSource::login` 的语义是「确保拿到一个可用会话」：在线实现在内部静默 refresh，
refresh 不可用时返回错误，由调用方的交互式登录流程重新授权，而不是在启动游戏时突然要求
用户去浏览器操作。

### 离线账户

离线账户的 uuid 由 `md5("OfflinePlayer:" + name)` 按 RFC-4122 version 3 派生，与官方启动器
和离线服务端一致，换启动器或换服务端时玩家存档与插件数据仍能对上同一个玩家。

### 微软账户

设备代码登录只负责交互式授权：请求设备代码、展示给用户、轮询令牌并换取 Xbox / Minecraft
会话。`MicrosoftAccountSource` 只做静默刷新，从调用方提供的 `RefreshTokenStore` 读取
uuid + refresh token；凭据缺失、不属于配置里的档案或已失效时按「尚未登录 / 登录已过期」
返回错误。Azure 客户端 ID 通过编译期环境变量 `DMCL_MICROSOFT_CLIENT_ID` 嵌入。

这个 crate 不直接依赖系统钥匙串：应用层用 `keyring` 实现 `RefreshTokenStore`，测试与无
桌面环境可以换成内存实现。访问 Microsoft / Xbox 服务的 User-Agent 由 crate 自身按 workspace
版本拼成 `dmcl/{version}`，不需要反向依赖启动器。
