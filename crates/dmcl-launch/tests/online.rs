//! 联网冒烟测试（默认 `#[ignore]`）：
//!
//! ```text
//! cargo test -p dmcl-launch --test online -- --ignored --nocapture
//! ```

use std::path::PathBuf;
use std::sync::Arc;

use dmcl_install::InstallOptions;
use dmcl_launch::{GameLauncher, LaunchOption, OfflineAccount};
use dmcl_mcmeta::source::{DownloadSource, Mirror};

const VERSION: &str = "1.7.10";

fn work_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("dmcl-launch-test-{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[tokio::test]
#[ignore = "requires network access"]
async fn integrity_and_command_line_for_a_real_version() {
    let root = work_dir("launch");
    let install_options = InstallOptions::new(&root, DownloadSource::new(Mirror::Official));
    dmcl_install::download_vanilla(&install_options, VERSION, VERSION)
        .await
        .expect("download_vanilla");

    let option = LaunchOption::new(Arc::new(OfflineAccount::new("player")), &root, VERSION);
    let mut launcher = GameLauncher::new(option).await.expect("GameLauncher");

    // 只装了版本 JSON：显然还不完整，而且能指出缺的是哪个 jar
    let status = launcher.check_integrity().await.expect("check_integrity");
    assert!(!status.is_complete(), "只装了元数据，不该被认为完整");
    assert!(status.missing_count() > 0);
    assert!(
        status
            .missing
            .iter()
            .any(|entry| entry.path.to_string_lossy().ends_with("1.7.10.jar")),
        "应当报告缺少客户端 jar：{:?}",
        status.detail(5)
    );
    // 缺失项带着下载用的模板地址，交给 dmcl-install 就能补
    assert!(status.missing.iter().any(|entry| entry
        .url
        .as_deref()
        .unwrap_or("")
        .starts_with("${")));

    // 命令行能生成出来（不要求文件齐，只是拼参数）
    let command = match launcher.generate_command().await {
        Ok(command) => command,
        Err(err) if format!("{err:#}").contains("java") => return, // 这台机器没有 java 就跳过
        Err(err) => panic!("generate_command: {err:#}"),
    };
    assert!(command.contains("--username player"), "{command}");
    assert!(command.contains("-Xmx2048m"), "{command}");
    // 所有模板都必须被替换掉
    assert!(!command.contains("${"), "命令里还有未替换的模板: {command}");
}
