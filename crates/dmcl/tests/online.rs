//! 联网冒烟测试（默认 `#[ignore]`）：
//!
//! ```text
//! cargo test -p dmcl --test online -- --ignored --nocapture
//! ```
//!
//! 这里走的是"配置 → 各 crate 选项 → 安装 / 启动"这条完整链路，
//! 也就是 CLI 每个命令实际走的那条路。

use std::path::PathBuf;

use dmcl::config::{AccountConfig, Config};
use dmcl_launch::GameLauncher;

const VERSION: &str = "1.7.10";

fn work_dir() -> PathBuf {
    let dir = std::env::temp_dir().join("dmcl-app-online-test");
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn config_for(root: PathBuf) -> Config {
    let mut config = Config::default();
    config.minecraft_roots.insert("test".to_string(), root);
    config.minecraft_root = "test".to_string();
    config.download.max_tasks = 8;
    config.download.chunk_threshold_mib = 1;
    config.accounts.insert(
        "player".to_string(),
        AccountConfig::Offline {
            username: "player".to_string(),
        },
    );
    config.launch.account = Some("player".to_string());
    config
}

#[tokio::test]
#[ignore = "requires network access"]
async fn the_whole_chain_runs_from_a_config_document() {
    let root = work_dir();
    let config = config_for(root.clone());

    // 配置 → 下载源 / 安装选项
    let source = config.download_source();
    let install = config
        .install_options(source, &config.launch, &root)
        .expect("install_options");
    assert_eq!(install.root_path, root);
    assert_eq!(install.download.max_connections, 8);

    // 原版元数据落盘（CLI 的 install 第一步）
    dmcl_install::download_vanilla(&install, VERSION, VERSION)
        .await
        .expect("download_vanilla");
    assert!(root
        .join(format!("versions/{VERSION}/{VERSION}.json"))
        .is_file());

    // 配置 → 启动选项 → 启动器
    let option = config
        .launch_option(&config.launch, VERSION, &root)
        .expect("launch_option");
    assert_eq!(option.account_source.name(), "player");
    let account = option.account_source.login().await.expect("login");
    let mut launcher = GameLauncher::new(option, account)
        .await
        .expect("GameLauncher");

    // 只装了元数据，完整性检查必须报告缺件，并且给出可用来修复的模板地址
    let status = dmcl_install::check_integrity(&install.plan(), VERSION)
        .await
        .expect("check_integrity");
    assert!(!status.is_complete());
    assert!(status.missing_count() > 0);
    assert!(status.summary().contains("需要修复"));
    assert!(
        status
            .missing
            .iter()
            .any(|entry| entry.url.as_deref().unwrap_or("").starts_with("${")),
        "缺失项应当带着 URL 模板，方便交给 dmcl-install 修复"
    );

    // 命令行能生成（这台机器没有 java 就跳过这一段）
    match launcher.generate_command().await {
        Ok(command) => {
            assert!(command.contains("--username player"), "{command}");
            assert!(!command.contains("${"), "还有未替换的模板: {command}");
        }
        Err(err) if format!("{err:#}").contains("java") => {}
        Err(err) => panic!("generate_command: {err:#}"),
    }
}

#[tokio::test]
#[ignore = "requires network access"]
async fn mirror_policy_reaches_the_source_mirrors() {
    use dmcl_mcmeta::source::{Mirror, MirrorPolicy};

    let root = work_dir();
    let mut config = config_for(root);

    // 策略 → 下载源的镜像优先级列表
    config.download.mirror = MirrorPolicy::OfficialOnly;
    assert_eq!(config.download_source().mirrors(), &[Mirror::Official]);

    config.download.mirror = MirrorPolicy::MirrorFirst;
    assert_eq!(
        config.download_source().mirrors(),
        &[Mirror::Bmclapi, Mirror::Official]
    );

    // 配置里的策略也确实能取到版本列表。
    // 注意：bmclapi 在部分网络环境下不可达（其跳转目标被代理拦截），
    // 因此这里用仅官方验证"配置 → 取数"这条链路。
    config.download.mirror = MirrorPolicy::OfficialOnly;
    let manifest = config
        .download_source()
        .fetch_version_manifest()
        .await
        .expect("manifest");
    assert!(manifest.latest_release().is_some());
}
