//! 微软账户：设备代码登录、refresh token 刷新，以及 Xbox / Minecraft 会话。
//!
//! 登录分两段：
//!
//! * [`device_code_login`] 是交互式授权，只在交互式登录场景里调用：展示一次性代码，
//!   轮询 Microsoft 登录服务，成功后再换 Xbox / Minecraft 会话；
//! * [`MicrosoftAccountSource`] 是启动链使用的账户源：只读取持久化的凭据
//!   （uuid + refresh token）并静默刷新，不会在启动游戏时突然要求用户去浏览器授权。
//!
//! refresh token 怎么存由调用方通过 [`RefreshTokenStore`] 决定；这个 crate 不直接
//! 依赖系统钥匙串，测试与无桌面环境也能换成内存实现。

use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use dmcl_task::BoxFuture;
use serde::{Deserialize, Serialize};

use crate::account::{Account, AccountSource};
use crate::LAUNCHER_VERSION;

/// 编译时嵌入的 Microsoft Entra 公共客户端 ID。
///
/// 构建时设置环境变量 `DMCL_MICROSOFT_CLIENT_ID`。未设置时仍可编译，只是微软登录
/// 会返回一条说明怎么重新构建的错误。
pub const CLIENT_ID: Option<&str> = option_env!("DMCL_MICROSOFT_CLIENT_ID");

const DEVICE_CODE_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode";
const TOKEN_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/token";
const SCOPE: &str = "XboxLive.signin offline_access";
const XBOX_LIVE_URL: &str = "https://user.auth.xboxlive.com/user/authenticate";
const XSTS_URL: &str = "https://xsts.auth.xboxlive.com/xsts/authorize";
const MINECRAFT_LOGIN_URL: &str =
    "https://api.minecraftservices.com/authentication/login_with_xbox";
const MINECRAFT_PROFILE_URL: &str = "https://api.minecraftservices.com/minecraft/profile";

/// 设备代码流要展示给用户的信息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceCodePrompt {
    /// 用户在验证页面输入的短代码。
    pub user_code: String,
    /// 验证页面地址。
    pub verification_uri: String,
    /// 代码有效期（秒）。
    pub expires_in: u64,
}

/// 由调用方决定怎么把设备代码告诉用户：CLI 打印到终端，GUI 以后可以弹窗。
pub trait DeviceCodePresenter: Send + Sync {
    fn present(&self, prompt: &DeviceCodePrompt);
}

/// 一次交互式设备代码登录的结果。
#[derive(Clone)]
pub struct DeviceCodeLogin {
    /// 可启动的账户快照。
    pub account: Account,
    /// 需要由调用方保存的 refresh token；下一次启动靠它静默刷新。
    pub refresh_token: String,
}

impl std::fmt::Debug for DeviceCodeLogin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceCodeLogin")
            .field("account", &self.account)
            .field("refresh_token", &"<已隐藏>")
            .finish()
    }
}

/// 持久化的微软凭据。
///
/// 除了 refresh token 还记住它属于哪个 Minecraft 档案：账户键被删掉又复用给另一个
/// 微软账户时，靠 uuid 认出旧凭据，而不是静默登录成上一个账户。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredCredential {
    /// 该凭据对应的 Minecraft 档案 uuid。
    pub uuid: String,
    /// 用来换新会话的 refresh token。
    pub refresh_token: String,
}

/// 微软凭据的持久化。
///
/// 应用层用系统钥匙串实现；测试或没有钥匙串的环境可以换成内存实现。
pub trait RefreshTokenStore: Send + Sync {
    fn load<'a>(&'a self) -> BoxFuture<'a, Result<Option<StoredCredential>>>;
    fn save<'a>(&'a self, credential: &'a StoredCredential) -> BoxFuture<'a, Result<()>>;
}

/// 启动链使用的微软账户源：只做静默刷新。
///
/// `login` 与 `refresh` 的语义相同：读取持久化的凭据换一份新的 Minecraft 会话。
/// 没有凭据、凭据不属于配置里的档案或已失效时返回错误 —— 启动游戏不应该变成一次
/// 浏览器交互。
#[derive(Clone)]
pub struct MicrosoftAccountSource {
    name: String,
    uuid: String,
    store: Arc<dyn RefreshTokenStore>,
    http: reqwest::Client,
}

impl MicrosoftAccountSource {
    /// `name` 是配置里的账户键，用于日志和错误提示；`uuid` 是配置里记录的玩家
    /// uuid，用来确认钥匙串里的凭据属于同一个档案。
    pub fn new(
        name: impl Into<String>,
        uuid: impl Into<String>,
        store: Arc<dyn RefreshTokenStore>,
    ) -> Self {
        Self {
            name: name.into(),
            uuid: uuid.into(),
            store,
            http: http_client(),
        }
    }
}

impl AccountSource for MicrosoftAccountSource {
    fn name(&self) -> &str {
        &self.name
    }

    fn login<'a>(&'a self) -> BoxFuture<'a, Result<Account>> {
        self.refresh()
    }

    fn refresh<'a>(&'a self) -> BoxFuture<'a, Result<Account>> {
        Box::pin(async move {
            let credential = self.store.load().await?;
            let credential = match credential {
                Some(credential) if same_profile(&credential.uuid, &self.uuid) => credential,
                _ => return Err(anyhow!("账户「{}」尚未登录", self.name)),
            };

            match refresh_session(&self.http, &credential.refresh_token).await {
                Ok(session) => {
                    if let Some(token) = session.rotated_refresh_token {
                        let credential = StoredCredential {
                            uuid: session.account.uuid.clone(),
                            refresh_token: token,
                        };
                        self.store.save(&credential).await?;
                    }
                    Ok(session.account)
                }
                Err(SessionError::InvalidGrant) => {
                    Err(anyhow!("账户「{}」的登录已过期", self.name))
                }
                Err(SessionError::Other(err)) => Err(err),
            }
        })
    }
}

/// 判断两个 uuid 是否指向同一个档案：忽略大小写与连字符。
///
/// 配置里手写的 uuid 可能带连字符，而 Minecraft 档案接口返回的是 32 位无连字符形式。
fn same_profile(a: &str, b: &str) -> bool {
    fn normalize(uuid: &str) -> String {
        uuid.chars()
            .filter(|ch| *ch != '-')
            .flat_map(char::to_lowercase)
            .collect()
    }
    normalize(a) == normalize(b)
}

/// 走设备代码流完成一次微软登录。
///
/// 成功后返回账户快照与 refresh token。**保存 token、写入配置由调用方负责**：
/// 账户键可能在拿到玩家名之后才能确定。
pub async fn device_code_login(presenter: &dyn DeviceCodePresenter) -> Result<DeviceCodeLogin> {
    let client_id = client_id()?;
    let client = http_client();

    let code = request_device_code(&client, client_id).await?;
    presenter.present(&DeviceCodePrompt {
        user_code: code.user_code.clone(),
        verification_uri: code.verification_uri.clone(),
        expires_in: code.expires_in,
    });

    let token = poll_device_code(&client, client_id, &code).await?;
    let refresh_token = token
        .refresh_token
        .ok_or_else(|| anyhow!("Microsoft 登录响应缺少 refresh token，无法保持登录状态"))?;
    let account = authenticate_minecraft(&client, &token.access_token).await?;
    Ok(DeviceCodeLogin {
        account,
        refresh_token,
    })
}

fn client_id() -> Result<&'static str> {
    CLIENT_ID.ok_or_else(|| {
        anyhow!(
            "当前构建没有内置 Microsoft 应用 ID。请在构建时设置 DMCL_MICROSOFT_CLIENT_ID 后重新编译，例如：\n  DMCL_MICROSOFT_CLIENT_ID=<client-id> cargo build --release"
        )
    })
}

fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(format!("dmcl/{LAUNCHER_VERSION}"))
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Microsoft 登录服务（OAuth 2.0 设备代码流）
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    #[serde(default = "default_poll_interval")]
    interval: u64,
}

fn default_poll_interval() -> u64 {
    5
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OAuthError {
    error: String,
    #[serde(default)]
    error_description: Option<String>,
}

/// 令牌端点返回的 OAuth 错误；轮询时 `authorization_pending` / `slow_down` 是正常状态。
#[derive(Debug)]
struct OAuthFailure {
    code: String,
    description: Option<String>,
}

impl OAuthFailure {
    fn network(err: reqwest::Error) -> Self {
        Self {
            code: "network".to_string(),
            description: Some(err.to_string()),
        }
    }

    fn into_anyhow(self) -> anyhow::Error {
        let detail = self.description.unwrap_or_default();
        match self.code.as_str() {
            "invalid_grant" => anyhow!("登录凭据已失效：{detail}"),
            "invalid_client" => {
                anyhow!("Microsoft 应用 ID 无效或没有启用公共客户端流程：{detail}")
            }
            "network" => anyhow!("无法连接 Microsoft 登录服务：{detail}"),
            "invalid_response" => {
                anyhow!("Microsoft 登录服务返回了无法解析的响应：{detail}")
            }
            other => anyhow!("Microsoft 登录失败（{other}）：{detail}"),
        }
    }
}

fn parse_oauth_error(body: &str) -> Option<OAuthFailure> {
    let error: OAuthError = serde_json::from_str(body).ok()?;
    if error.error.is_empty() {
        return None;
    }
    Some(OAuthFailure {
        code: error.error,
        description: error.error_description,
    })
}

async fn request_device_code(
    client: &reqwest::Client,
    client_id: &str,
) -> Result<DeviceCodeResponse> {
    let response = client
        .post(DEVICE_CODE_URL)
        .form(&[("client_id", client_id), ("scope", SCOPE)])
        .send()
        .await
        .context("无法连接 Microsoft 登录服务")?;
    let status = response.status();
    let body = response
        .text()
        .await
        .context("无法读取 Microsoft 登录服务的响应")?;

    if let Some(error) = parse_oauth_error(&body) {
        return Err(error.into_anyhow());
    }
    if !status.is_success() {
        return Err(anyhow!("Microsoft 登录服务返回 HTTP {status}：{body}"));
    }
    serde_json::from_str(&body).context("无法解析 Microsoft 设备代码响应")
}

async fn request_token(
    client: &reqwest::Client,
    form: &[(&str, &str)],
) -> std::result::Result<TokenResponse, OAuthFailure> {
    let response = client
        .post(TOKEN_URL)
        .form(form)
        .send()
        .await
        .map_err(OAuthFailure::network)?;
    let status = response.status();
    let body = response.text().await.map_err(OAuthFailure::network)?;

    if let Some(error) = parse_oauth_error(&body) {
        return Err(error);
    }
    if !status.is_success() {
        return Err(OAuthFailure {
            code: format!("http_{}", status.as_u16()),
            description: Some(body),
        });
    }
    serde_json::from_str(&body).map_err(|err| OAuthFailure {
        code: "invalid_response".to_string(),
        description: Some(format!("{err}: {body}")),
    })
}

async fn poll_device_code(
    client: &reqwest::Client,
    client_id: &str,
    code: &DeviceCodeResponse,
) -> Result<TokenResponse> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(code.expires_in);
    let mut interval = code.interval.max(1);

    loop {
        tokio::time::sleep(Duration::from_secs(interval)).await;
        if tokio::time::Instant::now() >= deadline {
            return Err(anyhow!("设备代码已过期"));
        }

        let result = request_token(
            client,
            &[
                ("client_id", client_id),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("device_code", &code.device_code),
            ],
        )
        .await;

        match result {
            Ok(token) => return Ok(token),
            Err(error) if error.code == "authorization_pending" => {}
            Err(error) if error.code == "slow_down" => interval += 5,
            Err(error) if error.code == "expired_token" => return Err(anyhow!("设备代码已过期")),
            Err(error) if error.code == "authorization_declined" => {
                return Err(anyhow!("登录请求已被拒绝"))
            }
            Err(error) => return Err(error.into_anyhow()),
        }
    }
}

// ---------------------------------------------------------------------------
// Xbox Live / XSTS / Minecraft
// ---------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct XblRequest {
    properties: XblProperties,
    relying_party: &'static str,
    token_type: &'static str,
}

#[derive(Serialize)]
struct XblProperties {
    #[serde(rename = "AuthMethod")]
    auth_method: &'static str,
    #[serde(rename = "SiteName")]
    site_name: &'static str,
    #[serde(rename = "RpsTicket")]
    rps_ticket: String,
}

#[derive(Deserialize)]
struct XblResponse {
    #[serde(rename = "Token")]
    token: String,
    #[serde(rename = "DisplayClaims")]
    display_claims: XblDisplayClaims,
}

#[derive(Deserialize)]
struct XblDisplayClaims {
    #[serde(rename = "xui")]
    xui: Vec<XblXui>,
}

#[derive(Deserialize)]
struct XblXui {
    #[serde(rename = "uhs")]
    uhs: String,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct XstsRequest {
    properties: XstsProperties,
    relying_party: &'static str,
    token_type: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct XstsProperties {
    sandbox_id: &'static str,
    user_tokens: Vec<String>,
}

#[derive(Deserialize)]
struct XstsResponse {
    #[serde(rename = "Token")]
    token: String,
}

#[derive(Deserialize)]
struct XstsError {
    #[serde(rename = "XErr")]
    xerr: i64,
}

#[derive(Serialize)]
struct MinecraftLoginRequest {
    #[serde(rename = "identityToken")]
    identity_token: String,
}

#[derive(Deserialize)]
struct MinecraftTokenResponse {
    access_token: String,
}

#[derive(Debug, Deserialize)]
struct MinecraftProfile {
    id: String,
    name: String,
}

/// 用 refresh token 换一份新的 Minecraft 会话。
struct RefreshedSession {
    account: Account,
    /// Microsoft 会轮换 refresh token；有新的才需要写回。
    rotated_refresh_token: Option<String>,
}

enum SessionError {
    /// refresh token 已失效，需要重新走交互式登录。
    InvalidGrant,
    Other(anyhow::Error),
}

async fn refresh_session(
    client: &reqwest::Client,
    refresh_token: &str,
) -> std::result::Result<RefreshedSession, SessionError> {
    let client_id = client_id().map_err(SessionError::Other)?;
    let token = request_token(
        client,
        &[
            ("client_id", client_id),
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("scope", SCOPE),
        ],
    )
    .await
    .map_err(|error| {
        if error.code == "invalid_grant" {
            SessionError::InvalidGrant
        } else {
            SessionError::Other(error.into_anyhow())
        }
    })?;

    let account = authenticate_minecraft(client, &token.access_token)
        .await
        .map_err(SessionError::Other)?;
    Ok(RefreshedSession {
        account,
        rotated_refresh_token: token.refresh_token.filter(|token| token != refresh_token),
    })
}

async fn authenticate_minecraft(
    client: &reqwest::Client,
    msa_access_token: &str,
) -> Result<Account> {
    let (xbl_token, uhs) = authenticate_xbox_live(client, msa_access_token).await?;
    let xsts_token = authorize_xsts(client, &xbl_token).await?;
    let minecraft_token = login_minecraft(client, &uhs, &xsts_token).await?;
    let profile = fetch_profile(client, &minecraft_token).await?;

    Ok(Account {
        username: profile.name,
        uuid: profile.id,
        access_token: minecraft_token,
        user_type: "msa".to_string(),
    })
}

async fn authenticate_xbox_live(
    client: &reqwest::Client,
    msa_access_token: &str,
) -> Result<(String, String)> {
    let request = XblRequest {
        properties: XblProperties {
            auth_method: "RPS",
            site_name: "user.auth.xboxlive.com",
            rps_ticket: format!("d={msa_access_token}"),
        },
        relying_party: "http://auth.xboxlive.com",
        token_type: "JWT",
    };

    let response = client
        .post(XBOX_LIVE_URL)
        .json(&request)
        .send()
        .await
        .context("无法连接 Xbox Live 认证服务")?;
    let status = response.status();
    let body = response
        .text()
        .await
        .context("无法读取 Xbox Live 认证响应")?;
    if !status.is_success() {
        return Err(anyhow!("Xbox Live 认证失败（HTTP {status}）：{body}"));
    }

    let response: XblResponse =
        serde_json::from_str(&body).context("无法解析 Xbox Live 认证响应")?;
    let uhs = response
        .display_claims
        .xui
        .into_iter()
        .next()
        .map(|xui| xui.uhs)
        .ok_or_else(|| anyhow!("Xbox Live 认证响应缺少用户哈希（uhs）"))?;
    Ok((response.token, uhs))
}

async fn authorize_xsts(client: &reqwest::Client, xbl_token: &str) -> Result<String> {
    let request = XstsRequest {
        properties: XstsProperties {
            sandbox_id: "RETAIL",
            user_tokens: vec![xbl_token.to_string()],
        },
        relying_party: "rp://api.minecraftservices.com/",
        token_type: "JWT",
    };

    let response = client
        .post(XSTS_URL)
        .json(&request)
        .send()
        .await
        .context("无法连接 Xbox 授权服务")?;
    let status = response.status();
    let body = response.text().await.context("无法读取 Xbox 授权响应")?;
    if !status.is_success() {
        if let Ok(error) = serde_json::from_str::<XstsError>(&body) {
            return Err(xsts_error(error.xerr));
        }
        return Err(anyhow!("Xbox 授权失败（HTTP {status}）：{body}"));
    }

    let response: XstsResponse = serde_json::from_str(&body).context("无法解析 Xbox 授权响应")?;
    Ok(response.token)
}

fn xsts_error(xerr: i64) -> anyhow::Error {
    let hint = match xerr {
        2148916233 => {
            "该 Microsoft 账户还没有 Xbox 档案；请先打开 https://www.xbox.com 登录并创建档案"
        }
        2148916235 => "Xbox Live 在账户所在的国家 / 地区不可用",
        2148916236 | 2148916237 => "该账户需要先在 Xbox 网站完成成人验证",
        2148916238 => "该账户是未成年人账户，需要先加入 Microsoft 家庭组",
        _ => "未知的 Xbox 授权错误",
    };
    anyhow!("Xbox 授权失败（XErr {xerr}）：{hint}")
}

async fn login_minecraft(client: &reqwest::Client, uhs: &str, xsts_token: &str) -> Result<String> {
    let request = MinecraftLoginRequest {
        identity_token: format!("XBL3.0 x={uhs};{xsts_token}"),
    };

    let response = client
        .post(MINECRAFT_LOGIN_URL)
        .json(&request)
        .send()
        .await
        .context("无法连接 Minecraft 登录服务")?;
    let status = response.status();
    let body = response
        .text()
        .await
        .context("无法读取 Minecraft 登录响应")?;
    if !status.is_success() {
        return Err(anyhow!("Minecraft 登录失败（HTTP {status}）：{body}"));
    }

    let response: MinecraftTokenResponse =
        serde_json::from_str(&body).context("无法解析 Minecraft 登录响应")?;
    Ok(response.access_token)
}

async fn fetch_profile(client: &reqwest::Client, access_token: &str) -> Result<MinecraftProfile> {
    let response = client
        .get(MINECRAFT_PROFILE_URL)
        .bearer_auth(access_token)
        .send()
        .await
        .context("无法连接 Minecraft 档案服务")?;
    let status = response.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return Err(anyhow!(
            "该 Microsoft 账户没有购买 Minecraft，或 Minecraft 档案不可用"
        ));
    }
    let body = response
        .text()
        .await
        .context("无法读取 Minecraft 档案响应")?;
    if !status.is_success() {
        return Err(anyhow!("获取 Minecraft 档案失败（HTTP {status}）：{body}"));
    }
    serde_json::from_str(&body).context("无法解析 Minecraft 档案响应")
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoTokenStore;

    impl RefreshTokenStore for NoTokenStore {
        fn load<'a>(&'a self) -> BoxFuture<'a, Result<Option<StoredCredential>>> {
            Box::pin(async { Ok(None) })
        }

        fn save<'a>(&'a self, _credential: &'a StoredCredential) -> BoxFuture<'a, Result<()>> {
            Box::pin(async { Ok(()) })
        }
    }

    struct FixedStore(StoredCredential);

    impl RefreshTokenStore for FixedStore {
        fn load<'a>(&'a self) -> BoxFuture<'a, Result<Option<StoredCredential>>> {
            Box::pin(async { Ok(Some(self.0.clone())) })
        }

        fn save<'a>(&'a self, _credential: &'a StoredCredential) -> BoxFuture<'a, Result<()>> {
            Box::pin(async { Ok(()) })
        }
    }

    #[test]
    fn oauth_errors_keep_the_code_and_description() {
        let error =
            parse_oauth_error(r#"{"error":"invalid_grant","error_description":"token expired"}"#)
                .expect("应当解析出错误");
        assert_eq!(error.code, "invalid_grant");
        assert_eq!(error.description.as_deref(), Some("token expired"));

        assert!(parse_oauth_error(r#"{"access_token":"x"}"#).is_none());
        assert!(parse_oauth_error("not json").is_none());
    }

    #[test]
    fn token_responses_parse_refresh_token() {
        let token: TokenResponse =
            serde_json::from_str(r#"{"access_token":"a","refresh_token":"r","expires_in":3600}"#)
                .unwrap();
        assert_eq!(token.access_token, "a");
        assert_eq!(token.refresh_token.as_deref(), Some("r"));

        let token: TokenResponse = serde_json::from_str(r#"{"access_token":"a"}"#).unwrap();
        assert!(token.refresh_token.is_none());
    }

    #[test]
    fn device_code_response_defaults_the_poll_interval() {
        let code: DeviceCodeResponse = serde_json::from_str(
            r#"{
                "device_code": "d",
                "user_code": "ABCD",
                "verification_uri": "https://microsoft.com/devicelogin",
                "expires_in": 900
            }"#,
        )
        .unwrap();
        assert_eq!(code.interval, 5);
        assert_eq!(code.expires_in, 900);
    }

    #[test]
    fn xbox_responses_expose_the_user_hash() {
        let response: XblResponse = serde_json::from_str(
            r#"{
                "Token": "t",
                "DisplayClaims": { "xui": [{ "uhs": "u" }] }
            }"#,
        )
        .unwrap();
        assert_eq!(response.token, "t");
        assert_eq!(response.display_claims.xui[0].uhs, "u");
    }

    #[test]
    fn minecraft_profile_parses_id_and_name() {
        let profile: MinecraftProfile =
            serde_json::from_str(r#"{"id":"069a79f444e94726a5befca90e38aaf5","name":"Steve"}"#)
                .unwrap();
        assert_eq!(profile.id, "069a79f444e94726a5befca90e38aaf5");
        assert_eq!(profile.name, "Steve");
    }

    #[test]
    fn xsts_errors_explain_common_account_problems() {
        assert!(xsts_error(2148916233).to_string().contains("Xbox 档案"));
        assert!(xsts_error(2148916238).to_string().contains("家庭组"));
    }

    #[tokio::test]
    async fn a_missing_credential_means_the_account_is_not_logged_in() {
        let source = MicrosoftAccountSource::new("steve", "uuid", Arc::new(NoTokenStore));
        let error = source.login().await.unwrap_err();
        assert_eq!(error.to_string(), "账户「steve」尚未登录");
    }

    #[tokio::test]
    async fn a_credential_from_another_profile_counts_as_not_logged_in() {
        let store = FixedStore(StoredCredential {
            uuid: "069a79f444e94726a5befca90e38aaf5".to_string(),
            refresh_token: "token".to_string(),
        });
        let source = MicrosoftAccountSource::new("steve", "other-uuid", Arc::new(store));
        let error = source.login().await.unwrap_err();
        assert_eq!(error.to_string(), "账户「steve」尚未登录");
    }

    #[test]
    fn uuids_match_regardless_of_case_and_hyphens() {
        assert!(same_profile(
            "069A79F4-44E9-4726-A5BE-FCA90E38AAF5",
            "069a79f444e94726a5befca90e38aaf5"
        ));
        assert!(!same_profile(
            "069a79f444e94726a5befca90e38aaf5",
            "5627dd98e6be3c21b8a8e92344183641"
        ));
    }
}
