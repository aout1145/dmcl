//! Account abstraction (was `dmcl.account`).

pub mod offline;

pub use offline::OfflineAccount;

/// The `Account` interface of the D original.
///
/// It is a trait object so that online (Microsoft/Yggdrasil) accounts can be
/// added later without touching the launcher.
pub trait Account: Send + Sync {
    /// Player name sent to the game (`${auth_player_name}`).
    fn get_name(&self) -> &str;
    /// Player uuid (`${auth_uuid}` / `${uuid}`).
    fn get_uuid(&self) -> String;
    /// Session token (`${auth_access_token}` / `${auth_session}`).
    fn get_access_token(&self) -> String;
    /// Account type (`${user_type}`).
    fn get_type(&self) -> &str;
}
