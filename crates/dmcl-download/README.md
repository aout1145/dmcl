# dmcl-download

并发下载文件：多文件并发 + 大文件分块并发 + 进度上报。`DownloadTask` 本身就是一个
`dmcl_task::Task`，可以直接塞进任务链，由前端订阅事件流显示整体进度。

`dmcl-download` 在 workspace 内只依赖 `dmcl-task`；哈希与校验实现都在它自己里面。
它不读配置：并发数、分块阈值、重试次数、UA 等全部由调用者通过 `DownloadOptions` 传入。

## 模块布局

```text
task.rs     DownloadTask 本体：调度、分块、重试、进度
item.rs     DownloadItem：候选地址、目标路径、校验值、大小、显示名
options.rs  DownloadOptions 与默认 User-Agent
hash.rs     Checksum / HashAlgorithm，以及 file_ok / files_ok
report.rs   DownloadReport / DownloadFailure
```

## 设计要点

| 关注点       | 做法                                                                                                                                                                                         |
| ------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 多文件并发   | `JoinSet` + 全局 `Semaphore(max_connections)`                                                                                                                                                |
| 大文件分块   | 已知大小且超过阈值时，先发 `bytes=0-0` 探针确认服务器支持 `Range`（返回 206 + `Content-Range`），再按块并发；单文件并发块数另有限制                                                          |
| 不支持 Range | 自动退回单连接                                                                                                                                                                               |
| 完整性       | 先写 `<文件>.part`，校验通过才 `rename`；失败删掉半成品并重试                                                                                                                                |
| 校验算法     | `Checksum` 支持 **md5 / sha1 / sha256 / sha512**（Minecraft 文档给的是 sha1、Fabric 给的是 sha512；也接受 `sha256:…` 前缀或按摘要长度推断）                                                 |
| 进度         | 已知大小的项按字节、未知大小的项按数量（字节「下载多少算多少」），整体百分比按文件数占比加权；总量按规划大小预填，拿到服务器实际大小（`Content-Length` / `Content-Range`）后修订、失败项回滚；带节流 |
| 换源重试     | 每个下载项带一串**候选地址**（由 `dmcl-install` 按镜像策略展开）；第 `n` 次尝试轮换一个（A B A…），全失败才记为失败                                                                        |
| 断点续传     | 不做；失败即整份重来（最多 `max_attempts` 次）                                                                                                                                               |
| 失败处理     | 所有已排队项目都会跑完，失败项进 `DownloadReport`；默认「有失败就报错」，可用 `DownloadOptions::ignore_failures` 改成「只记录不报错」                                                        |

补充说明：

- **本地判断也并发**：下载前的「已存在」检查与完整性检查都走 `files_ok`，按 CPU 线程数
  并发哈希，不会串行扫几千个资源文件；
- **共享连接池**：`dmcl-install` 通过 `InstallOptions::download_task` 复用下载源的
  `reqwest::Client`，安装与下载共用同一个连接池；
- **进度单位**：`Progress.current` / `total` 的单位由下载任务自己定义（字节或文件数），
  百分比只看 `Progress.percent`。

## 相关文档

- 总体结构与依赖关系：[ARCHITECTURE.md](../../ARCHITECTURE.md)
- 任务链与事件流：[crates/dmcl-task/README.md](../dmcl-task/README.md)
- 谁构造下载项、如何按镜像展开候选地址：[crates/dmcl-install/README.md](../dmcl-install/README.md)
