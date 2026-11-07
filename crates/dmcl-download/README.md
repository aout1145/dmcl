# dmcl-download

单文件下载任务 + 共享并发预算 + 全局进度。每个文件是一个独立的 `dmcl_task::Task`，由下载
准备节点动态加入任务图；下载层只负责单文件，失败由 DAG FailFast 处理。

`dmcl-download` 在 workspace 内只依赖 `dmcl-task`；哈希与校验实现都在它自己里面。它不读
配置：并发数、分块阈值、重试次数、UA 等全部由调用者通过 `DownloadOptions` 传入。

## 模块布局

```text
task.rs     DownloadContext / 单文件任务：预检、分块、重试、进度、信号量
item.rs     DownloadItem：候选地址、目标路径、校验值、大小、显示名
options.rs  DownloadOptions 与默认 User-Agent
hash.rs     Checksum / HashAlgorithm，以及 file_ok / files_ok / files_ok_with
```

## 设计要点

| 关注点       | 做法                                                                                                                                 |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------------ |
| 文件任务     | `DownloadContext::task(item)` 返回一个 `Task<(), ()>`；失败直接返回错误，由 DAG FailFast 处理                                        |
| 准备节点     | `DownloadContext::prepare(ctx, step, items)`：预检 → `add_total` → `ctx.add_task` 动态创建文件任务（任务名为「下载 <文件名>」）；返回文件句柄，需要读文件时可 `wait` |
| 三层并发     | `max_active_files`（文件）→ `max_connections`（HTTP / 分块）→ `max_workers`（预检 / 校验）；获取顺序固定                              |
| 大文件分块   | 已知大小且超过阈值时，先发 `bytes=0-0` 探针确认 `Range`；每块占一个全局连接许可，单文件可占满全部连接                                  |
| 不支持 Range | 自动退回单连接                                                                                                                       |
| 完整性       | 先写 `<文件>.part`，校验通过才 `rename`；失败删掉半成品并重试                                                                        |
| 校验算法     | `Checksum` 支持 **md5 / sha1 / sha256 / sha512**                                                                                     |
| 进度         | 全局只有 `downloaded / total` 字节；预检只计入缺失文件的已知大小，服务器实际大小到达后修订；重试扣回本轮已报告字节                   |
| 换源重试     | 每个下载项带一串**候选地址**；第 `n` 次尝试轮换一个（A B A…），全失败才让节点失败                                                   |
| 断点续传     | 不做；失败即整份重来（首次之外最多重试 `max_retries` 次）                                                                            |
| 取消         | 信号量获取、HTTP 请求、读响应体、哈希校验都可取消；取消时 permit 随 future 释放                                                      |

共享连接池：`DownloadContext::new` 接收调用方的 `reqwest::Client`；业务层用下载源的
client 建上下文，安装 / 启动图里的多次下载复用同一份预算与全局进度。

## 相关文档

- 总体结构与依赖关系：[ARCHITECTURE.md](../../ARCHITECTURE.md)
- 任务图、Step 与事件：[crates/dmcl-task/README.md](../dmcl-task/README.md)
- 谁构造下载项、如何按镜像展开候选地址：[crates/dmcl-install/README.md](../dmcl-install/README.md)
