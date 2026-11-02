//! `login` —— 微软账户设备代码登录。

use anyhow::{anyhow, Result};
use dmcl_launch::microsoft::{
    device_code_login, DeviceCodePresenter, DeviceCodePrompt, RefreshTokenStore, StoredCredential,
};

use crate::config::{unique_account_name, AccountConfig, Config, KeyringTokenStore};

/// 把设备代码打印到终端。
struct TerminalPresenter;

impl DeviceCodePresenter for TerminalPresenter {
    fn present(&self, prompt: &DeviceCodePrompt) {
        println!();
        println!(
            "请在浏览器打开 {}，输入代码：{}",
            prompt.verification_uri, prompt.user_code
        );
        println!(
            "代码 {} 秒内有效；登录完成后会自动继续。",
            prompt.expires_in
        );
        println!();
    }
}

/// `dmcl login [name]`。
///
/// 省略 `name` 时用登录后的玩家名作为账户键；该键已存在就加后缀，绝不覆盖。
/// 显式给出 `name` 时才覆盖同名条目。
pub(crate) async fn run(name: Option<&str>, config: &mut Config) -> Result<()> {
    if name.is_some_and(|name| name.trim().is_empty()) {
        return Err(anyhow!("账户名不能为空"));
    }

    // 先确认凭据库可用，别让用户走完浏览器流程才发现 token 存不了。
    KeyringTokenStore::ensure_available().await?;

    let login = device_code_login(&TerminalPresenter).await?;

    let key = match name {
        Some(name) => name.to_string(),
        None => unique_account_name(&config.accounts, &login.account.username),
    };

    // 先写钥匙串：失败时配置保持不变，不会留下一个指向空凭据的账户。
    KeyringTokenStore::new(&key)
        .save(&StoredCredential {
            uuid: login.account.uuid.clone(),
            refresh_token: login.refresh_token,
        })
        .await?;

    config.accounts.insert(
        key.clone(),
        AccountConfig::Microsoft {
            username: login.account.username.clone(),
            uuid: login.account.uuid.clone(),
        },
    );
    let selected = config.launch.account.is_none();
    if selected {
        config.launch.account = Some(key.clone());
    }
    config.save().await?;

    println!("已登录：{}（账户「{key}」）", login.account.username);
    if selected {
        println!("已设为当前账户。");
    }
    Ok(())
}
