# dmcl-download

单文件下载任务 + 共享并发预算 + 全局进度。每个文件是一个独立的 `dmcl_task::Task`，由下载
准备节点动态加入任务图；下载层只负责单文件，失败由 DAG FailFast 处理。

`dmcl-download` 在 workspace 内只依赖 `dmcl-task`。它不读配置：并发数、分块阈值、重试
次数、UA 等全部由调用者通过 `DownloadOptions` 传入。

## 模块布局

```text
task.rs     DownloadContext / 单文件任务：预检、分块、重试、进度、信号量
item.rs     DownloadItem：候选地址、目标路径、校验值、大小、显示名
options.rs  DownloadOptions 与默认 User-Agent
hash.rs     Checksum / HashAlgorithm，以及 file_ok / files_ok / files_ok_with
```

## 设计意图

- **准备与执行分离**：`DownloadContext::prepare` 做预检、计入全局总量，再把每个文件动态
  加入任务图；下载任务本身只处理一个目标，失败由任务图统一处理。
- **三层并发预算**：文件、HTTP 连接 / 分块、预检 / 校验分别受限，由共享的
  `DownloadContext` 统一分配，安装与启动图里的多次下载复用同一份预算和进度。
- **大文件分块**：已知大小且超过阈值时分块下载，不支持 Range 时自动退回单连接。
- **完整性优先**：先写临时文件，校验通过才改名；失败删掉半成品并重试。
- **候选地址轮换**：每个下载项带一串候选地址，重试时轮换，全部失败才让节点失败；重试按
  指数退避，并尽量遵循服务器的限流响应。
- **同路径去重**：同一目标路径的下载任务串行化，后到者在拿到锁后重新检查，已完整就跳过。
- **可取消**：信号量、HTTP 请求、响应体读取与校验都可取消，取消时占用的许可随 future
  释放。
- **不做断点续传**：失败即整份重来，保持实现简单。

完整性校验支持 md5 / sha1 / sha256 / sha512；没有下载地址的条目仍参与本地检查，缺失时由
上层报错。

## 相关文档

- 总体结构与依赖关系：[ARCHITECTURE.md](../../ARCHITECTURE.md)
- 任务图、Step 与事件：[crates/dmcl-task/README.md](../dmcl-task/README.md)
- 谁构造下载项、如何按镜像展开候选地址：[crates/dmcl-install/README.md](../dmcl-install/README.md)
