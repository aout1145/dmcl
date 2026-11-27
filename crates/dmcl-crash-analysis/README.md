# dmcl-crash-analysis

崩溃原因分析：按日志特征判断游戏异常退出的可能原因，给出结论与建议。

纯文本分析，**不读文件、不依赖 GTK**，也不依赖 workspace 内的其它 crate。调用方把会话日志
与崩溃报告拼成文本传进来，再把 `Finding` 显示在界面上。

## 模块布局

```text
lib.rs    公开 API：Exit / Finding、分层调度、崩溃报告定位
exit.rs   退出码 / 信号，以及退出方式本身能给出的结论
rules.rs  规则表、文案与提取函数
```

## 规则表

`rules.rs` 的 `RULES` 一行一条规则：特征串、所属层、结论。固定结论直接写一行
（`Message::Fixed`）；需要从日志里取 mod 名、版本号的才指向提取函数（`Message::Handler`）。
新增规则通常只加一行，不用再写一个「定义正则 + 调用正则」的包装函数。

## 设计意图

### 三层优先级

规则按「先精确、后模糊」分成三层，命中一层就不再往下猜：

1. 加载器层：缺少前置、重复安装、mod 冲突、Mixin 失败、mod 加载崩溃、配置损坏；
2. 运行时层：内存、Java 版本、最大内存、显卡驱动、原生崩溃、权限、网络，以及退出方式；
3. 泛化层：`NoClassDefFoundError` 之类的版本不匹配，以及实体 / 方块崩溃。

加载器解析失败后，后续的缺类错误基本都是衍生信息；把两层混在一起会把主因判断成
`NoClassDefFoundError`。

### 只认证据，不猜

规则只匹配日志里确实出现的报错格式，宁可给不出结论也不输出泛化猜测。重复出现的同一结论
按 `cause` 去重；mod 名、文件名、需要的 Java 版本等从日志里截取，超长片段截断，避免异常
日志把提示卡片撑爆。

### 退出方式也是证据

`0xC0000005`、`SIGSEGV`、`SIGABRT` 这类退出方式没有对应日志时也要能给出结论，所以 `Exit`
与日志分析一起放在这里，由调用方从 `ExitStatus` 构造。

## 使用

```rust
let mut text = log.clone();
if let Some(path) = dmcl_crash_analysis::crash_report_path(&log) {
    // 读取崩溃报告并追加到 text
}
let findings = dmcl_crash_analysis::analyze(&text, &exit);
```

`crash_report_path` 解析日志末尾的 `#@!@# Game crashed! Crash report saved to: #@!@#` 标记；
读取文件与相对路径解析由调用方负责。
