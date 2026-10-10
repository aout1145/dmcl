//! Offline (cracked) account (was `dmcl.account.offline`).

use sha1::{Digest, Sha1};

use super::Account;

/// An account without any authentication.
#[derive(Debug, Clone)]
pub struct OfflineAccount {
    pub name: String,
}

impl OfflineAccount {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

impl Account for OfflineAccount {
    fn get_name(&self) -> &str {
        &self.name
    }

    fn get_uuid(&self) -> String {
        sha1_uuid(&self.name)
    }

    fn get_access_token(&self) -> String {
        String::new()
    }

    fn get_type(&self) -> &str {
        "legacy"
    }
}

/// `std.uuid.sha1UUID(name).toString()`: a name-based (version 5) RFC-4122 uuid
/// built from the first 16 bytes of `sha1(name)`.
///
/// The D version flipped the version/variant bits and formatted the hex by hand;
/// `uuid::Builder::from_sha1_bytes` does precisely that, so the resulting string
/// is byte-identical for every name.
pub fn sha1_uuid(name: &str) -> String {
    let mut hasher = Sha1::new();
    hasher.update(name.as_bytes());
    let digest = hasher.finalize();

    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[0..16]);
    uuid::Builder::from_sha1_bytes(bytes)
        .into_uuid()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offline_uuid_is_deterministic_and_shaped_like_v5() {
        let uuid = sha1_uuid("player");
        assert_eq!(uuid.len(), 36);
        assert_eq!(&uuid[14..15], "5"); // version 5
                                        // RFC-4122 variant: the nibble is 0b10xx, i.e. one of 8/9/a/b
        assert!(
            ["8", "9", "a", "b"].contains(&&uuid[19..20]),
            "unexpected variant nibble in {uuid}"
        );
        assert_eq!(uuid, sha1_uuid("player"));
        assert_ne!(uuid, sha1_uuid("other"));
    }

    /// Golden values produced by the original D algorithm (`sha1(name)[0..16]`
    /// with the version/variant bits set), so swapping the hand-written bit
    /// fiddling for `uuid::Builder` provably changed nothing.
    #[test]
    fn offline_uuid_matches_the_d_implementation_byte_for_byte() {
        assert_eq!(sha1_uuid("player"), "889c6853-a117-5ca8-bef9-d6523335dc06");
        assert_eq!(sha1_uuid("Steve"), "2121f63f-a284-5c8a-9ce4-00bb6f83824f");
        assert_eq!(sha1_uuid(""), "da39a3ee-5e6b-5b0d-b255-bfef95601890");
    }
}
