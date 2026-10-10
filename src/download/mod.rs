//! Download subsystem (was `dmcl.download`).

pub mod fabric;
pub mod forge;
pub mod game;
pub mod mirrors;
pub mod multitasks;
pub mod neoforge;
mod processors;

pub use fabric::{get_latest_fabric, install_fabric, show_fabric_version_list};
pub use forge::{get_latest_forge, install_forge, show_forge_version_list};
pub use game::{
    download_assets, download_game_files, download_libraries, download_vanilla,
    generate_launcher_profiles, show_version_list,
};
pub use mirrors::mirror_url;
pub use multitasks::{download_file, wait_downloads, DownloadFileMeta};
pub use neoforge::{get_latest_neoforge, install_neoforge, show_neoforge_version_list};
