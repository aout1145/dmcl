# dmcl-mcmeta

集中描述「游戏是什么」：Minecraft 的强类型元数据文档、下载源与 URL 模板、本地目录布局。
它**不把文件写到磁盘上**，也不读配置文件；根目录、镜像、并发度等一律由调用者传入。

`dmcl-mcmeta` 是 workspace 最底层的叶子 crate，不依赖其他成员 crate。外部依赖主要是
`serde` / `serde_json` / `reqwest` / `tokio`；可选的 `clap` feature 让 `Mirror` 直接作为
clap 的候选值，供前端复用。

## 模块布局

```text
vanilla/     version.rs  manifest.rs  asset.rs  rules.rs  urls.rs
fabric/      类型 + 地址 + fetch
forge/       mod.rs + install_profile.rs
neoforge/    mod.rs + install_profile.rs
source.rs    镜像、URL 模板、DownloadSource
os.rs        Os / Arch 与平台约定
maven.rs     maven 坐标 → 仓库相对路径
merge.rs     递归合并两份 JSON
layout.rs    .minecraft 目录布局、读取版本 JSON（含 inheritsFrom）
json.rs      JsonMap 别名
macros.rs    string_enum! 内部宏
```

## 设计要点

### 强类型文档

版本 JSON、版本清单、资源索引、安装器档案、模组加载器版本列表都是强类型结构体。
会被写回的文档必须能往返：

- 会被写回的结构体都带 `#[serde(flatten)] extra: JsonMap`，未知字段原样保留；
- 「已知取值 + 兜底」的字符串枚举用 `string_enum!` 宏，且**必须保留 `Other(String)`**
  —— 否则回写时会丢掉不认识的取值；
- 可选字段用 `#[serde(default, skip_serializing_if = "Option::is_none")]`，别写出 `null`。

### URL 模板与下载源

从 Minecraft 拿到的每一份文档里，所有 url 都会被**归一成模板**：

```text
https://libraries.minecraft.net/net/sf/jopt-simple/…/jopt-simple-5.0.4.jar
        ↓
${MC_MAVEN}/net/sf/jopt-simple/…/jopt-simple-5.0.4.jar
```

`${MC_MAVEN}` 是一个「下载根」（`Root`），真正的地址由 `Mirror` 决定：

```rust
let source = DownloadSource::from_policy(MirrorPolicy::MirrorFirst);
source.resolve("${MC_MAVEN}/a/b.jar");
// → https://bmclapi2.bangbang93.com/maven/a/b.jar
```

要点：

- 归一化选**最长前缀**匹配（否则 bmclapi 的站点根会把 `/maven` 之类的更具体的根吞掉）；
- 一个根可以有**多个等价官域**（Forge 的 `files.minecraftforge.net/maven` 与
  `maven.minecraftforge.net` 是同一个仓库，都归一成 `${FORGE_MAVEN}`）；
- 识别不出的 host 原样保留 = 不可镜像；
- `UrlFields` trait 统一遍历文档里的 url 字段，`templatize_urls` / `resolve_urls`
  对任何文档类型都通用，新增字段不会漏改；
- 用哪种镜像由 `MirrorPolicy` 决定：`official-only`（仅官方）、`official-first`
  （官方优先，默认）、`mirror-first`（镜像优先）；各域取 meta 统一走 `fetch_meta(url)`
  —— 内部按镜像优先级展开候选、轮换重试（A B A …）直到用满重试次数，
  `fetch_json` 就是它的 JSON 版：
  `source.fetch_json("${MC_META}/mc/game/version_manifest.json").await?`。

写回磁盘的版本 JSON 会把模板解析回**官方**地址，因此 `.minecraft` 里的文档与官方启动器
完全通用。

### rules 判定

`vanilla/rules.rs` 按**标准 Mojang 语义**判定：空规则允许；否则按顺序评估、**最后一条
命中的规则的 action 决定结果**；一条都不命中则不允许。条件涵盖 `os.name` / `os.arch` /
`os.version`（正则）/ `os.versionRange`（区间）/ `features`。

### 加载器版本列表

Fabric / Forge / NeoForge 的 `fetch_*` 统一按版本**从新到旧**返回：Fabric / NeoForge
比版本字符串，Forge 比 `build` 号；因此「最新」就是列表第一条，调用方不再需要单独的
`latest_*` 取数函数。

### 本地目录布局

`layout.rs` 约定 `versions/<name>/<name>.json`、`libraries/`、`assets/` 等路径，以及
natives 解压目录（`<临时目录>/dmcl-natives-<os>-<arch>-<版本名哈希>`），并提供读取版本
JSON（含 `inheritsFrom` 合并）的入口。这里只算路径、读文档，不写盘；真正的落盘在
`dmcl-install`。

## 相关文档

- 总体结构与依赖关系：[ARCHITECTURE.md](../../ARCHITECTURE.md)
- 用这些元数据规划并落盘：[crates/dmcl-install/README.md](../dmcl-install/README.md)
- 下载任务如何消费候选地址：[crates/dmcl-download/README.md](../dmcl-download/README.md)
