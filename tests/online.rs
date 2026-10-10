//! Online smoke tests.
//!
//! These hit the real Mojang endpoints, so they are `#[ignore]`d by default.
//! Run them with:
//!
//! ```text
//! cargo test --test online -- --ignored --nocapture
//! ```

use std::path::PathBuf;

use dmcl::download::mirrors::Mirror;

fn test_root() -> PathBuf {
    std::env::temp_dir().join("dmcl-online-test")
}

#[tokio::test]
#[ignore = "requires network access"]
async fn download_vanilla_writes_the_version_json() {
    let root = test_root();

    dmcl::download::download_vanilla(Mirror::Official, &root, "1.7.10", "1.7.10")
        .await
        .expect("download_vanilla");

    let expected = root.join("versions/1.7.10/1.7.10.json");
    assert!(expected.is_file(), "missing {}", expected.display());

    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&expected).unwrap()).unwrap();
    assert_eq!(json["id"], "1.7.10");
    assert!(json["libraries"].is_array());

    // fields the launcher does not model must survive the typed round-trip
    for key in ["time", "releaseTime", "type", "minimumLauncherVersion"] {
        assert!(json.get(key).is_some(), "lost {key}");
    }

    // `generateLauncherProfiles` must have created this as well
    assert!(root.join("launcher_profiles.json").is_file());
}

#[tokio::test]
#[ignore = "requires network access"]
async fn version_list_can_be_queried() {
    dmcl::download::show_version_list(Mirror::Official, &["release".to_string()])
        .await
        .expect("show_version_list");
}

#[tokio::test]
#[ignore = "requires network access"]
async fn fabric_version_list_can_be_queried() {
    dmcl::download::show_fabric_version_list(Mirror::Official, "1.20.1")
        .await
        .expect("show_fabric_version_list");
}
