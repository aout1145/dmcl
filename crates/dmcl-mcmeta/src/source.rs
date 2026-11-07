//! 下载源：镜像源枚举、URL 模板、以及"按模板发 http 请求"的客户端。
//!
//! # URL 模板
//!
//! 所有从 Minecraft 文档里读出来的链接都会被**归一成模板**，例如：
//!
//! ```text
//! https://libraries.minecraft.net/net/sf/jopt-simple/jopt-simple/5.0.4/jopt-simple-5.0.4.jar
//! ${MC_MAVEN}/net/sf/jopt-simple/jopt-simple/5.0.4/jopt-simple-5.0.4.jar
//! ```
//!
//! `${MC_MAVEN}` 叫一个 [`Root`]（下载根）。真正下载时由 [`Root::base`] 按当前
//! [`Mirror`] 决定换成官方地址还是镜像地址，于是"下什么"和"从哪儿下"彻底分开：
//! 安装流程只跟模板打交道，换镜像不需要改一行安装逻辑。
//!
//! 一个镜像可以只覆盖部分下载根（例如 ForgeCDN 只镜像 `${NEOFORGE_MAVEN}`）。
//! 展开候选地址时会跳过不覆盖该根的镜像，因此部分镜像不会把官方地址提前。
//!
//! 识别不出来的地址（第三方仓库等）保持原样，也就是**不可镜像**。

use std::fmt;
use std::str::FromStr;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// 默认 User-Agent，未显式设置时使用。
pub const DEFAULT_USER_AGENT: &str = concat!("dmcl/", env!("CARGO_PKG_VERSION"));

// ---------------------------------------------------------------------------
// 镜像源
// ---------------------------------------------------------------------------

/// 使用哪个镜像源。
///
/// 镜像可以只覆盖部分下载根（见 [`Root::mirror_bases`]），例如 ForgeCDN 只镜像
/// NeoForge 的 maven 仓库。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mirror {
    #[default]
    Official,
    Bmclapi,
    ForgeCdn,
}

// ---------------------------------------------------------------------------
// 镜像使用策略
// ---------------------------------------------------------------------------

/// 下载源的镜像使用策略。
///
/// 只暴露三种固定选择，配置里不用写任意镜像列表：
/// 仅官方、官方优先（失败再试镜像）、镜像优先（失败再回官方）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MirrorPolicy {
    /// 仅使用官方源。
    OfficialOnly,
    /// 官方源优先，失败再试镜像源。
    #[default]
    OfficialFirst,
    /// 镜像源优先，失败再回官方源。
    MirrorFirst,
}

impl MirrorPolicy {
    /// 全部策略。
    pub const ALL: [MirrorPolicy; 3] = [
        MirrorPolicy::OfficialOnly,
        MirrorPolicy::OfficialFirst,
        MirrorPolicy::MirrorFirst,
    ];

    /// 配置文件与命令行里使用的名字。
    pub fn as_str(self) -> &'static str {
        match self {
            MirrorPolicy::OfficialOnly => "official-only",
            MirrorPolicy::OfficialFirst => "official-first",
            MirrorPolicy::MirrorFirst => "mirror-first",
        }
    }

    /// 展开成镜像优先级列表（靠前的先试）。
    ///
    /// 部分镜像不覆盖的根会被跳过（见 [`resolve_url_candidates`]），所以
    /// `ForgeCdn` 排在官方之前也不会影响 `${NEOFORGE_MAVEN}` 以外的地址。
    pub fn mirrors(self) -> Vec<Mirror> {
        match self {
            MirrorPolicy::OfficialOnly => vec![Mirror::Official],
            MirrorPolicy::OfficialFirst => {
                vec![Mirror::Official, Mirror::Bmclapi, Mirror::ForgeCdn]
            }
            MirrorPolicy::MirrorFirst => {
                vec![Mirror::Bmclapi, Mirror::ForgeCdn, Mirror::Official]
            }
        }
    }
}

impl fmt::Display for MirrorPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for MirrorPolicy {
    type Err = anyhow::Error;

    /// 不认识的值直接报错（配置文件写错时能立刻发现）。
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "official-only" => Ok(MirrorPolicy::OfficialOnly),
            "official-first" => Ok(MirrorPolicy::OfficialFirst),
            "mirror-first" => Ok(MirrorPolicy::MirrorFirst),
            other => Err(anyhow!(
                "不认识的镜像策略：{other}（可选 official-only / official-first / mirror-first）"
            )),
        }
    }
}

impl Serialize for MirrorPolicy {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for MirrorPolicy {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        raw.parse().map_err(serde::de::Error::custom)
    }
}

// ---------------------------------------------------------------------------
// 下载根（URL 模板里的占位符）
// ---------------------------------------------------------------------------

/// URL 模板里的一个"根"。每个根在各镜像下对应一个（或几个等价的）base url。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Root {
    /// Mojang 的版本元数据（`piston-meta`）。
    McMeta,
    /// Mojang 的版本文件（`piston-data`，新版客户端 jar 在这里）。
    McData,
    /// 旧的 `launcher.mojang.com` 资源。
    McLauncher,
    /// 官方库仓库（libraries）。
    McMaven,
    /// 资源文件（assets objects）。
    McAssets,
    /// Forge 的 maven 仓库。
    ForgeMaven,
    /// NeoForge 的 maven 仓库。
    NeoForgeMaven,
    /// Fabric 的元数据接口。
    FabricMeta,
    /// Fabric 的 maven 仓库。
    FabricMaven,
}

impl Root {
    /// 全部下载根。
    pub const ALL: [Root; 9] = [
        Root::McMeta,
        Root::McData,
        Root::McLauncher,
        Root::McMaven,
        Root::McAssets,
        Root::ForgeMaven,
        Root::NeoForgeMaven,
        Root::FabricMeta,
        Root::FabricMaven,
    ];

    /// 模板里的占位符，例如 `${MC_MAVEN}`。
    pub fn placeholder(self) -> &'static str {
        match self {
            Root::McMeta => "${MC_META}",
            Root::McData => "${MC_DATA}",
            Root::McLauncher => "${MC_LAUNCHER}",
            Root::McMaven => "${MC_MAVEN}",
            Root::McAssets => "${MC_ASSETS}",
            Root::ForgeMaven => "${FORGE_MAVEN}",
            Root::NeoForgeMaven => "${NEOFORGE_MAVEN}",
            Root::FabricMeta => "${FABRIC_META}",
            Root::FabricMaven => "${FABRIC_MAVEN}",
        }
    }

    /// 官方镜像下的 base url。
    ///
    /// 返回切片是因为同一个仓库可能有多个等价域名（Forge 的
    /// `files.minecraftforge.net/maven` 与 `maven.minecraftforge.net` 就是同一个仓库）。
    pub fn official_bases(self) -> &'static [&'static str] {
        match self {
            Root::McMeta => &["https://piston-meta.mojang.com"],
            Root::McData => &["https://piston-data.mojang.com"],
            Root::McLauncher => &["https://launcher.mojang.com"],
            Root::McMaven => &["https://libraries.minecraft.net"],
            Root::McAssets => &["https://resources.download.minecraft.net"],
            Root::ForgeMaven => &[
                "https://files.minecraftforge.net/maven",
                "https://maven.minecraftforge.net",
            ],
            Root::NeoForgeMaven => &["https://maven.neoforged.net/releases"],
            Root::FabricMeta => &["https://meta.fabricmc.net"],
            Root::FabricMaven => &["https://maven.fabricmc.net"],
        }
    }

    /// `mirror` 真正镜像的该根的 base url；`None` 表示该镜像不镜像这个根。
    ///
    /// 需要"解析出一个可用地址"时用 [`Root::bases`]（它会回落官方）。
    pub fn mirror_bases(self, mirror: Mirror) -> Option<&'static [&'static str]> {
        match mirror {
            Mirror::Official => Some(self.official_bases()),
            Mirror::Bmclapi => match self {
                Root::McMeta => Some(&["https://bmclapi2.bangbang93.com"]),
                Root::McMaven => Some(&["https://bmclapi2.bangbang93.com/maven"]),
                Root::McAssets => Some(&["https://bmclapi2.bangbang93.com/assets"]),
                Root::ForgeMaven => Some(&["https://bmclapi2.bangbang93.com/maven"]),
                Root::NeoForgeMaven => Some(&["https://bmclapi2.bangbang93.com/maven"]),
                Root::FabricMeta => Some(&["https://bmclapi2.bangbang93.com/fabric-meta"]),
                Root::FabricMaven => Some(&["https://bmclapi2.bangbang93.com/maven"]),
                // bmclapi 不镜像这两个根（不改写）
                Root::McData | Root::McLauncher => None,
            },
            // ForgeCDN 只镜像 NeoForge 的 maven 仓库，其他根一律不覆盖。
            Mirror::ForgeCdn => match self {
                Root::NeoForgeMaven => Some(&["https://neoforged.forgecdn.net/releases"]),
                _ => None,
            },
        }
    }

    /// `mirror` 下该根的 base url；该镜像不镜像这个根时回落到官方地址。
    pub fn bases(self, mirror: Mirror) -> &'static [&'static str] {
        self.mirror_bases(mirror)
            .unwrap_or_else(|| self.official_bases())
    }

    /// `mirror` 下的主 base url。
    pub fn base(self, mirror: Mirror) -> &'static str {
        self.bases(mirror).first().copied().unwrap_or_default()
    }

    /// 由占位符反查根，例如 `"${MC_MAVEN}"` → [`Root::McMaven`]。
    pub fn from_placeholder(text: &str) -> Option<Root> {
        Root::ALL
            .into_iter()
            .find(|root| root.placeholder() == text)
    }
}

// ---------------------------------------------------------------------------
// 模板 <-> 真实 url
// ---------------------------------------------------------------------------

/// `url` 是否含有模板占位符。
pub fn is_templated(url: &str) -> bool {
    url.contains("${")
}

/// 把真实 url 归一成模板；识别不出 host 时原样返回（即"不可镜像"）。
///
/// 选择**最长**的匹配前缀，否则 `${MC_META}`（bmclapi 下就是站点根）会把
/// `${MC_MAVEN}` 之类的更具体的根吞掉。
pub fn templatize_url(url: &str) -> String {
    let mut best: Option<(usize, &'static str)> = None;
    for root in Root::ALL {
        for mirror in [Mirror::Official, Mirror::Bmclapi, Mirror::ForgeCdn] {
            let Some(bases) = root.mirror_bases(mirror) else {
                continue;
            };
            for base in bases {
                if !base.is_empty() && url.starts_with(base) {
                    let better = match best {
                        None => true,
                        Some((len, _)) => base.len() > len,
                    };
                    if better {
                        best = Some((base.len(), root.placeholder()));
                    }
                }
            }
        }
    }
    match best {
        Some((len, placeholder)) => format!("{placeholder}{}", &url[len..]),
        None => url.to_string(),
    }
}

/// 把模板里的占位符换成 `mirror` 下的真实 base。
///
/// 这是内部实现：下载地址一律通过 [`resolve_url_candidates`] 生成，写盘地址通过
/// [`resolve_official_urls`] 生成官方地址。
fn resolve_with_mirror(url: &str, mirror: Mirror) -> String {
    let mut out = url.to_string();
    if !is_templated(&out) {
        return out;
    }
    for root in Root::ALL {
        let placeholder = root.placeholder();
        if out.contains(placeholder) {
            out = out.replace(placeholder, root.base(mirror));
        }
    }
    out
}

/// `mirror` 是否覆盖 `url` 模板里出现的所有下载根。
///
/// 普通 url（不含占位符）视为不需要任何镜像，因此总返回 `true`。
fn mirror_covers(mirror: Mirror, url: &str) -> bool {
    Root::ALL
        .into_iter()
        .filter(|root| url.contains(root.placeholder()))
        .all(|root| root.mirror_bases(mirror).is_some())
}

/// 按镜像优先级把模板展开成候选 url（保序去重）。
///
/// 不覆盖模板里所有下载根的镜像会被跳过；一个都不覆盖时回落到官方地址。
/// 这样部分镜像（例如只镜像 NeoForge 的 ForgeCDN）不会把其他根的官方地址提前。
pub fn resolve_url_candidates(url: &str, mirrors: &[Mirror]) -> Vec<String> {
    if !is_templated(url) {
        return vec![url.to_string()];
    }
    let mut out: Vec<String> = Vec::new();
    for mirror in mirrors {
        if !mirror_covers(*mirror, url) {
            continue;
        }
        let resolved = resolve_with_mirror(url, *mirror);
        if !out.contains(&resolved) {
            out.push(resolved);
        }
    }
    if out.is_empty() {
        out.push(resolve_with_mirror(url, Mirror::Official));
    }
    out
}

/// 一个下载地址：既可能是不含占位符的普通 url，也可能是 `${MC_MAVEN}/…`。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UrlTemplate(String);

impl UrlTemplate {
    /// 直接包一层，不校验。普通 url 也允许（表示这个地址无法镜像）。
    pub fn new(template: impl Into<String>) -> Self {
        Self(template.into())
    }

    /// 由根与相对路径拼出模板。
    pub fn from_parts(root: Root, path: &str) -> Self {
        let path = path.strip_prefix('/').unwrap_or(path);
        Self(format!("{}/{}", root.placeholder(), path))
    }

    /// 校验模板：每个 `${…}` 都必须是已知的下载根。
    pub fn parse(template: &str) -> Result<Self> {
        let mut rest = template;
        while let Some(start) = rest.find("${") {
            let after = &rest[start..];
            let end = after
                .find('}')
                .ok_or_else(|| anyhow!("URL 模板缺少右花括号: {template}"))?;
            let placeholder = &after[..=end];
            if Root::from_placeholder(placeholder).is_none() {
                return Err(anyhow!("URL 模板里有未知的下载根: {placeholder}"));
            }
            rest = &after[end + 1..];
        }
        Ok(Self(template.to_string()))
    }

    /// 原始模板文本。
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// 是否含占位符（即是否可被镜像）。
    pub fn is_templated(&self) -> bool {
        is_templated(&self.0)
    }

    /// 模板里出现的下载根。
    pub fn roots(&self) -> Vec<Root> {
        Root::ALL
            .into_iter()
            .filter(|root| self.0.contains(root.placeholder()))
            .collect()
    }

    /// 按镜像优先级展开成候选链接。
    pub fn candidates(&self, mirrors: &[Mirror]) -> Vec<String> {
        resolve_url_candidates(&self.0, mirrors)
    }

    /// 由真实 url 反推模板；地址无法识别时返回 `None`。
    pub fn from_url(url: &str) -> Option<Self> {
        let templated = templatize_url(url);
        if templated == url {
            None
        } else {
            Some(Self(templated))
        }
    }
}

impl fmt::Display for UrlTemplate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for UrlTemplate {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

impl From<String> for UrlTemplate {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl Serialize for UrlTemplate {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for UrlTemplate {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self(String::deserialize(deserializer)?))
    }
}

// ---------------------------------------------------------------------------
// 文档里的 url 字段
// ---------------------------------------------------------------------------

/// 一个文档结构体里所有"下载地址"字段的遍历。
///
/// 有了它，[`templatize_urls`] / [`resolve_official_urls`] 就能对所有文档类型通用，
/// 新增字段时只需要在 `for_each_url_mut` 里补一行，不会漏。
pub trait UrlFields {
    /// 对每个 url 字段调用一次 `f`（可以就地改写）。
    fn for_each_url_mut(&mut self, f: &mut dyn FnMut(&mut String));
}

/// 读出文档里所有 url（先克隆再遍历，因此要求 `Clone`）。
pub fn urls_of<T: UrlFields + Clone>(value: &T) -> Vec<String> {
    let mut owned = value.clone();
    let mut out = Vec::new();
    owned.for_each_url_mut(&mut |url| out.push(url.clone()));
    out
}

/// 把文档里所有 url 归一成模板形式，返回改写的个数。
pub fn templatize_urls<T: UrlFields>(value: &mut T) -> usize {
    let mut changed = 0;
    value.for_each_url_mut(&mut |url| {
        let templated = templatize_url(url);
        if templated != *url {
            *url = templated;
            changed += 1;
        }
    });
    changed
}

/// 把文档里所有 url 模板解析成**官方**地址，返回改写的个数。
///
/// 写盘用的文档必须指向官方地址（镜像只在下载时使用），因此这里不接受 `Mirror`。
pub fn resolve_official_urls<T: UrlFields>(value: &mut T) -> usize {
    let mut changed = 0;
    value.for_each_url_mut(&mut |url| {
        if !is_templated(url) {
            return;
        }
        let resolved = resolve_with_mirror(url, Mirror::Official);
        if resolved != *url {
            *url = resolved;
            changed += 1;
        }
    });
    changed
}

// ---------------------------------------------------------------------------
// 下载源
// ---------------------------------------------------------------------------

/// 一个下载源：**镜像优先级列表** + http 客户端 + 单次请求的重试次数。
///
/// * 列表里第一个是首选镜像，请求失败（网络错误 / 404）时轮换到下一个，直到用满
///   [`DownloadSource::max_retries`] 次重试；
/// * 所有接受 url 的方法都同时接受"模板"与"普通 url"，
///   因此可以直接 `source.fetch_json("${MC_META}/mc/game/version_manifest.json")`。
#[derive(Debug, Clone)]
pub struct DownloadSource {
    mirrors: Vec<Mirror>,
    client: reqwest::Client,
    user_agent: String,
    max_retries: u32,
    retry_base_delay: Duration,
    retry_max_delay: Duration,
}

/// 单个请求默认的重试次数（与下载侧 `max_retries` 默认值一致）。
const DEFAULT_MAX_RETRIES: u32 = 3;
/// 第 1 次重试的基础等待时间。
const DEFAULT_RETRY_BASE_DELAY: Duration = Duration::from_secs(1);
/// 指数退避的上限。
const DEFAULT_RETRY_MAX_DELAY: Duration = Duration::from_secs(60);
/// 服务器要求等待的上限：`Retry-After` 再大也不真的睡那么久。
const MAX_RETRY_AFTER: Duration = Duration::from_secs(300);

/// 建立连接的超时。
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// 两次读取之间的空闲超时。
const READ_TIMEOUT: Duration = Duration::from_secs(30);

impl DownloadSource {
    /// 单一镜像。
    pub fn new(mirror: Mirror) -> Self {
        Self::build(vec![mirror], DEFAULT_USER_AGENT)
    }

    /// 官方源。
    pub fn official() -> Self {
        Self::new(Mirror::Official)
    }

    /// 按镜像策略构造（策略至少含一个镜像，因此不会失败）。
    pub fn from_policy(policy: MirrorPolicy) -> Self {
        Self::build(policy.mirrors(), DEFAULT_USER_AGENT)
    }

    /// 按优先级列表尝试多个镜像。列表不能为空。
    pub fn with_priority(mirrors: impl IntoIterator<Item = Mirror>) -> Result<Self> {
        let mirrors: Vec<Mirror> = {
            let mut out: Vec<Mirror> = Vec::new();
            for mirror in mirrors {
                if !out.contains(&mirror) {
                    out.push(mirror);
                }
            }
            out
        };
        if mirrors.is_empty() {
            return Err(anyhow!("下载源的镜像列表不能为空"));
        }
        Ok(Self::build(mirrors, DEFAULT_USER_AGENT))
    }

    /// 覆盖 User-Agent。
    pub fn with_user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = user_agent.into();
        self.client = build_client(&self.user_agent);
        self
    }

    /// 单次请求失败后最多重试几次（不含第一次尝试）。
    pub fn with_max_retries(mut self, retries: u32) -> Self {
        self.max_retries = retries;
        self
    }

    /// 设置指数退避：第 1 次重试等 `base`，之后翻倍，最长不超过 `max`。
    ///
    /// 服务器返回 `Retry-After` 时以响应头为准，不使用这里的退避。
    pub fn with_retry_delay(mut self, base: Duration, max: Duration) -> Self {
        self.retry_base_delay = base;
        self.retry_max_delay = max;
        self
    }

    fn build(mirrors: Vec<Mirror>, user_agent: &str) -> Self {
        Self {
            mirrors,
            client: build_client(user_agent),
            user_agent: user_agent.to_string(),
            max_retries: DEFAULT_MAX_RETRIES,
            retry_base_delay: DEFAULT_RETRY_BASE_DELAY,
            retry_max_delay: DEFAULT_RETRY_MAX_DELAY,
        }
    }

    /// 优先级最高的镜像。
    pub fn mirror(&self) -> Mirror {
        self.mirrors.first().copied().unwrap_or_default()
    }

    /// 镜像优先级列表。
    pub fn mirrors(&self) -> &[Mirror] {
        &self.mirrors
    }

    pub fn user_agent(&self) -> &str {
        &self.user_agent
    }

    /// 单次请求的最大重试次数。
    pub fn max_retries(&self) -> u32 {
        self.max_retries
    }

    /// 复用的 http 客户端（连接池 / keep-alive）。
    pub fn client(&self) -> &reqwest::Client {
        &self.client
    }

    /// 按优先级展开成候选链接。
    pub fn candidates(&self, url: impl AsRef<str>) -> Vec<String> {
        resolve_url_candidates(url.as_ref(), &self.mirrors)
    }

    /// 取一份 meta：`url` 可以是模板，按镜像优先级展开成候选、轮换重试（A B A …），
    /// 第一个成功的响应体（UTF-8 文本）即返回；**解析由调用方自己做**。
    pub async fn fetch_meta(&self, url: &str) -> Result<String> {
        let candidates = self.candidates(url);
        let mut last: Option<anyhow::Error> = None;
        let attempts = self.max_retries.saturating_add(1);
        for attempt in 1..=attempts {
            let candidate = &candidates[candidate_index(candidates.len(), attempt)];
            match self.fetch_text(candidate).await {
                Ok(body) => return Ok(body),
                Err(err) => {
                    // 429 等限流响应会给出明确的等待时间，优先照它等；
                    // 其它错误按指数退避，避免所有请求同时重试把源打满。
                    let retry_after = err
                        .downcast_ref::<HttpStatusError>()
                        .and_then(|error| error.retry_after);
                    last = Some(err.context(format!("从 {candidate} 取数失败")));
                    if attempt == attempts {
                        break;
                    }
                    let delay = retry_after
                        .map(|delay| delay.min(MAX_RETRY_AFTER))
                        .unwrap_or_else(|| self.retry_delay(attempt));
                    tokio::time::sleep(delay).await;
                }
            }
        }
        Err(last.unwrap_or_else(|| anyhow!("没有可用的下载地址：{url}")))
    }

    /// 第 `retry` 次重试（从 1 开始）的指数退避等待时间。
    fn retry_delay(&self, retry: u32) -> Duration {
        let factor = 1u32 << retry.saturating_sub(1).min(31);
        self.retry_base_delay
            .saturating_mul(factor)
            .min(self.retry_max_delay)
    }

    /// 取一份 JSON：`fetch_meta` + 反序列化。
    pub async fn fetch_json<T: DeserializeOwned>(&self, url: &str) -> Result<T> {
        let body = self.fetch_meta(url).await?;
        serde_json::from_str(&body).with_context(|| format!("解析 {url} 的 JSON 失败"))
    }

    /// 单次 GET（响应体按 UTF-8 文本返回）；重试由 [`DownloadSource::fetch_meta`] 负责。
    async fn fetch_text(&self, url: &str) -> Result<String> {
        let response = ensure_success(self.client.get(url).send().await?, url)?;
        let bytes = response.bytes().await?;
        String::from_utf8(bytes.to_vec()).with_context(|| format!("{url} 的响应不是合法 UTF-8"))
    }
}

/// 非 2xx 响应转成错误；429 等限流响应带上服务器要求的等待时间。
fn ensure_success(response: reqwest::Response, url: &str) -> Result<reqwest::Response> {
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status();
    let retry_after = retry_after(&response);
    Err(anyhow::Error::new(HttpStatusError {
        url: url.to_string(),
        status,
        retry_after,
    }))
}

/// 从响应头里取建议的重试等待时间。
///
/// 标准头 `Retry-After` 支持秒数；bmclapi 的 429 只给
/// `x-ratelimit-reset-short`（同样是秒数），作为兜底。
fn retry_after(response: &reqwest::Response) -> Option<Duration> {
    let parse = |header: Option<&reqwest::header::HeaderValue>| {
        header?.to_str().ok()?.trim().parse::<u64>().ok()
    };
    let header = response.headers().get(reqwest::header::RETRY_AFTER);
    let mut seconds = parse(header);
    if seconds.is_none() && response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
        seconds = parse(response.headers().get("x-ratelimit-reset-short"));
    }
    seconds.map(Duration::from_secs)
}

/// 非 2xx 响应；`retry_after` 来自 [`retry_after`]。
#[derive(Debug)]
struct HttpStatusError {
    url: String,
    status: reqwest::StatusCode,
    retry_after: Option<Duration>,
}

impl fmt::Display for HttpStatusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "服务器返回 {}（{}）", self.status, self.url)
    }
}

impl std::error::Error for HttpStatusError {}

/// 候选轮换：第 `attempt` 次（从 1 开始）用第几个候选（A B A …）。
fn candidate_index(len: usize, attempt: u32) -> usize {
    (attempt.saturating_sub(1) as usize) % len
}

fn build_client(user_agent: &str) -> reqwest::Client {
    // 配置里只有 UA 和超时，构建失败只可能是 TLS 后端无法初始化 —— 那种情况下
    // 任何取数都做不了，直接暴露出来，不要静默退化成没有超时的默认客户端。
    reqwest::Client::builder()
        .user_agent(user_agent)
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(READ_TIMEOUT)
        .build()
        .expect("无法初始化 HTTP 客户端")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirror_policy_names_round_trip_and_typos_are_rejected() {
        assert_eq!(MirrorPolicy::default(), MirrorPolicy::OfficialFirst);
        for policy in MirrorPolicy::ALL {
            let text = policy.as_str();
            assert_eq!(text.parse::<MirrorPolicy>().unwrap(), policy);
            let json = serde_json::to_string(&policy).unwrap();
            assert_eq!(json, format!("\"{text}\""));
            assert_eq!(serde_json::from_str::<MirrorPolicy>(&json).unwrap(), policy);
        }
        // 旧的单值写法（official / bmclapi）不再是合法策略
        assert!("official".parse::<MirrorPolicy>().is_err());
        assert!(serde_json::from_str::<MirrorPolicy>("\"typo\"").is_err());
    }

    #[test]
    fn official_bases_are_rewritten_into_templates() {
        assert_eq!(
            templatize_url("https://libraries.minecraft.net/a/b.jar"),
            "${MC_MAVEN}/a/b.jar"
        );
        assert_eq!(
            templatize_url("https://piston-meta.mojang.com/mc/game/version_manifest.json"),
            "${MC_META}/mc/game/version_manifest.json"
        );
        assert_eq!(
            templatize_url("https://resources.download.minecraft.net/ab/cdef"),
            "${MC_ASSETS}/ab/cdef"
        );
        assert_eq!(
            templatize_url("https://meta.fabricmc.net/v2/versions/loader/1.20.1"),
            "${FABRIC_META}/v2/versions/loader/1.20.1"
        );
        // Forge 的两个等价域名归一到同一个根
        assert_eq!(
            templatize_url("https://files.minecraftforge.net/maven/x.jar"),
            "${FORGE_MAVEN}/x.jar"
        );
        assert_eq!(
            templatize_url("https://maven.minecraftforge.net/x.jar"),
            "${FORGE_MAVEN}/x.jar"
        );
    }

    #[test]
    fn already_mirrored_urls_normalise_back_to_templates() {
        // 最长前缀优先：bmclapi 的 `/maven` 必须先于站点根被匹配
        assert_eq!(
            templatize_url("https://bmclapi2.bangbang93.com/maven/a/b.jar"),
            "${MC_MAVEN}/a/b.jar"
        );
        assert_eq!(
            templatize_url("https://bmclapi2.bangbang93.com/assets/ab/cdef"),
            "${MC_ASSETS}/ab/cdef"
        );
        assert_eq!(
            templatize_url("https://bmclapi2.bangbang93.com/mc/game/version_manifest.json"),
            "${MC_META}/mc/game/version_manifest.json"
        );
        assert_eq!(
            templatize_url("https://neoforged.forgecdn.net/releases/net/neoforged/neoforge/a.jar"),
            "${NEOFORGE_MAVEN}/net/neoforged/neoforge/a.jar"
        );
    }

    #[test]
    fn forgecdn_only_covers_the_neoforge_maven_root() {
        assert_eq!(
            resolve_url_candidates("${NEOFORGE_MAVEN}/a.jar", &[Mirror::ForgeCdn]),
            vec!["https://neoforged.forgecdn.net/releases/a.jar".to_string()]
        );
        // 不覆盖的根回落到官方
        assert_eq!(
            resolve_url_candidates("${MC_MAVEN}/a.jar", &[Mirror::ForgeCdn]),
            vec!["https://libraries.minecraft.net/a.jar".to_string()]
        );

        // MirrorFirst 是 [bmclapi, forgecdn, official]，ForgeCDN 只影响 NeoForge
        let mirrors = MirrorPolicy::MirrorFirst.mirrors();
        assert_eq!(
            resolve_url_candidates("${NEOFORGE_MAVEN}/a.jar", &mirrors),
            vec![
                "https://bmclapi2.bangbang93.com/maven/a.jar".to_string(),
                "https://neoforged.forgecdn.net/releases/a.jar".to_string(),
                "https://maven.neoforged.net/releases/a.jar".to_string(),
            ]
        );
        assert_eq!(
            resolve_url_candidates("${MC_MAVEN}/a.jar", &mirrors),
            vec![
                "https://bmclapi2.bangbang93.com/maven/a.jar".to_string(),
                "https://libraries.minecraft.net/a.jar".to_string(),
            ]
        );
    }

    #[test]
    fn partial_mirrors_do_not_promote_the_official_url() {
        // 即使部分镜像排在前面，候选里也不会出现“官方地址先于覆盖该根的镜像”
        let candidates = resolve_url_candidates(
            "${MC_MAVEN}/a.jar",
            &[Mirror::ForgeCdn, Mirror::Bmclapi, Mirror::Official],
        );
        assert_eq!(
            candidates,
            vec![
                "https://bmclapi2.bangbang93.com/maven/a.jar".to_string(),
                "https://libraries.minecraft.net/a.jar".to_string(),
            ]
        );

        // 没有任何镜像覆盖时回落官方
        let candidates = resolve_url_candidates("${MC_DATA}/x", &[Mirror::ForgeCdn]);
        assert_eq!(
            candidates,
            vec!["https://piston-data.mojang.com/x".to_string()]
        );
    }

    #[test]
    fn unknown_hosts_are_left_alone() {
        let url = "https://example.com/a.jar";
        assert_eq!(templatize_url(url), url);
        assert!(UrlTemplate::from_url(url).is_none());
    }

    #[test]
    fn templates_resolve_per_mirror() {
        let template = "${MC_MAVEN}/a/b.jar";
        assert_eq!(
            resolve_url_candidates(template, &[Mirror::Official]),
            vec!["https://libraries.minecraft.net/a/b.jar".to_string()]
        );
        assert_eq!(
            resolve_url_candidates(template, &[Mirror::Bmclapi]),
            vec!["https://bmclapi2.bangbang93.com/maven/a/b.jar".to_string()]
        );
        // 不被镜像的根回落到官方
        assert_eq!(
            resolve_url_candidates("${MC_DATA}/v1/objects/x/client.jar", &[Mirror::Bmclapi]),
            vec!["https://piston-data.mojang.com/v1/objects/x/client.jar".to_string()]
        );
        // 不含占位符时原样返回
        assert_eq!(
            resolve_url_candidates("https://example.com/a.jar", &[Mirror::Bmclapi]),
            vec!["https://example.com/a.jar".to_string()]
        );
    }

    #[test]
    fn candidates_are_ordered_and_deduplicated() {
        let candidates = resolve_url_candidates(
            "${MC_META}/mc/game/version_manifest.json",
            &[Mirror::Bmclapi, Mirror::Official],
        );
        assert_eq!(
            candidates,
            vec![
                "https://bmclapi2.bangbang93.com/mc/game/version_manifest.json".to_string(),
                "https://piston-meta.mojang.com/mc/game/version_manifest.json".to_string(),
            ]
        );

        // 同一个地址只出现一次
        let candidates =
            resolve_url_candidates("${MC_DATA}/x", &[Mirror::Bmclapi, Mirror::Official]);
        assert_eq!(candidates.len(), 1);
    }

    #[test]
    fn url_template_validates_its_roots() {
        assert!(UrlTemplate::parse("${MC_MAVEN}/a.jar").is_ok());
        assert!(UrlTemplate::parse("https://example.com/a.jar").is_ok());
        assert!(UrlTemplate::parse("${NOPE}/a.jar").is_err());
        assert!(UrlTemplate::parse("${MC_MAVEN/a.jar").is_err());

        let template = UrlTemplate::from_parts(Root::McAssets, "/ab/cdef");
        assert_eq!(template.as_str(), "${MC_ASSETS}/ab/cdef");
        assert_eq!(template.roots(), vec![Root::McAssets]);
        assert_eq!(
            template.candidates(&[Mirror::Bmclapi]),
            vec!["https://bmclapi2.bangbang93.com/assets/ab/cdef".to_string()]
        );
        assert!(template.is_templated());
        assert!(!UrlTemplate::new("https://example.com/a.jar").is_templated());
    }

    #[test]
    fn retry_delays_grow_exponentially_and_stop_at_the_cap() {
        let source = DownloadSource::new(Mirror::Official)
            .with_retry_delay(Duration::from_millis(100), Duration::from_millis(500));
        assert_eq!(source.retry_delay(1), Duration::from_millis(100));
        assert_eq!(source.retry_delay(2), Duration::from_millis(200));
        assert_eq!(source.retry_delay(3), Duration::from_millis(400));
        assert_eq!(source.retry_delay(4), Duration::from_millis(500));
        assert_eq!(source.retry_delay(u32::MAX), Duration::from_millis(500));
    }

    #[test]
    fn a_source_needs_at_least_one_mirror() {
        assert!(DownloadSource::with_priority([]).is_err());
        let source = DownloadSource::with_priority([Mirror::Bmclapi, Mirror::Official]).unwrap();
        assert_eq!(source.mirror(), Mirror::Bmclapi);
        assert_eq!(source.mirrors().len(), 2);
        // 重复的镜像会被去掉
        let source = DownloadSource::with_priority([Mirror::Bmclapi, Mirror::Bmclapi]).unwrap();
        assert_eq!(source.mirrors().len(), 1);
        assert_eq!(source.mirrors(), &[Mirror::Bmclapi]);
    }

    #[test]
    fn candidates_skip_mirrors_that_do_not_cover_the_root() {
        let source =
            DownloadSource::with_priority([Mirror::ForgeCdn, Mirror::Bmclapi, Mirror::Official])
                .unwrap();
        assert_eq!(
            source.candidates("${NEOFORGE_MAVEN}/a.jar"),
            vec![
                "https://neoforged.forgecdn.net/releases/a.jar".to_string(),
                "https://bmclapi2.bangbang93.com/maven/a.jar".to_string(),
                "https://maven.neoforged.net/releases/a.jar".to_string(),
            ]
        );
        assert_eq!(
            source.candidates("${MC_MAVEN}/a.jar"),
            vec![
                "https://bmclapi2.bangbang93.com/maven/a.jar".to_string(),
                "https://libraries.minecraft.net/a.jar".to_string(),
            ]
        );
        assert_eq!(
            source.candidates("${MC_DATA}/x"),
            vec!["https://piston-data.mojang.com/x".to_string()]
        );
    }

    #[test]
    fn generic_rewriting_visits_every_url_field() {
        use crate::vanilla::version::VersionJson;

        let json = serde_json::json!({
            "downloads": {
                "client": {"url": "https://piston-data.mojang.com/v1/objects/x/client.jar"}
            },
            "assetIndex": {"url": "https://piston-meta.mojang.com/v1/packages/x/1.json"},
            "libraries": [
                {"name": "a:a:1", "url": "https://maven.minecraftforge.net/",
                 "downloads": {"artifact": {"url": "https://libraries.minecraft.net/a.jar"}}}
            ]
        });
        let mut version: VersionJson = serde_json::from_value(json).unwrap();

        // 4 处：client jar、资源索引、库的仓库根、库的 artifact
        assert_eq!(templatize_urls(&mut version), 4);
        let urls = urls_of(&version);
        assert!(urls.contains(&"${MC_MAVEN}/a.jar".to_string()));
        assert!(urls.contains(&"${FORGE_MAVEN}/".to_string()));
        assert!(urls.contains(&"${MC_DATA}/v1/objects/x/client.jar".to_string()));
        // 未建模字段里的字符串不会被误改
        assert!(urls
            .iter()
            .any(|url| url == "${MC_META}/v1/packages/x/1.json"));

        // 写盘前统一解析回官方地址
        assert_eq!(resolve_official_urls(&mut version), 4);
        let urls = urls_of(&version);
        assert!(urls.contains(&"https://libraries.minecraft.net/a.jar".to_string()));
        assert!(urls.contains(&"https://files.minecraftforge.net/maven/".to_string()));
        // 不可镜像的地址保持原样
        assert!(
            urls.contains(&"https://piston-data.mojang.com/v1/objects/x/client.jar".to_string())
        );
    }
}
