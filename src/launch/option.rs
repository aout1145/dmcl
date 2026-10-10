//! Launch options (was `dmcl.launch.option`).

use std::path::PathBuf;
use std::sync::Arc;

use crate::account::Account;
use crate::env::LAUNCHER_NAME;

/// Everything needed to launch one version.
///
/// The account is stored as an `Arc<dyn Account>` (instead of a raw interface
/// reference) so the struct stays `Send + Sync` and can be shared with a
/// future GUI. Paths use Rust's own path types instead of `String`.
#[derive(Clone)]
pub struct LaunchOption {
    pub account: Arc<dyn Account>,
    /// Root of the minecraft installation (`.minecraft`), absolute.
    pub root_path: PathBuf,
    /// The *name* of the version directory, not a path.
    pub version_name: String,
    /// Window size in pixels and heap bounds in MiB — never negative.
    pub window_width: u32,
    pub window_height: u32,
    pub min_memory: u32,
    pub max_memory: u32,
    pub custom_info: String,
    pub additional_jvm: String,
    pub additional_game: String,
}

impl LaunchOption {
    /// Same defaults as the D struct for every field except the three that are
    /// always provided by the caller.
    pub fn new(
        account: Arc<dyn Account>,
        root_path: impl Into<PathBuf>,
        version_name: impl Into<String>,
    ) -> Self {
        Self {
            account,
            root_path: root_path.into(),
            version_name: version_name.into(),
            window_width: 854,
            window_height: 480,
            min_memory: 512,
            max_memory: 2048,
            custom_info: LAUNCHER_NAME.to_string(),
            additional_jvm:
                "-Dfml.ignoreInvalidMinecraftCertificates=True -Dfml.ignorePatchDiscrepancies=True"
                    .to_string(),
            additional_game: String::new(),
        }
    }
}
