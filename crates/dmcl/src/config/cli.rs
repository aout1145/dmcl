//! `cli.*` 段。

use serde::{Deserialize, Serialize};

/// `cli.*` 段。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CliConfig {
    pub default_command: String,
}

impl Default for CliConfig {
    fn default() -> Self {
        Self {
            default_command: "gui".to_string(),
        }
    }
}
