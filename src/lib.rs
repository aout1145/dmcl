//! dmcl — an unofficial Minecraft launcher.
//!
//! This crate is the *core* of the launcher. Everything that is not strictly
//! CLI-related (configuration, downloading, launching, account handling) lives
//! here, so that the current CLI front-end can be replaced or complemented by
//! a GUI front-end without touching the core logic.
//!
//! Module layout mirrors the original D implementation one-to-one, with one
//! addition: [`mcjson`] holds the typed views of Minecraft's JSON documents
//! (the D version used untyped `JSONValue` lookups everywhere).
//!
//! | original (D)        | this crate                  |
//! |---------------------|-----------------------------|
//! | `dmcl.config`       | [`config`]                  |
//! | `dmcl.env`          | [`env`]                     |
//! | `dmcl.utils`        | [`utils`]                   |
//! | `dmcl.account`      | [`account`]                 |
//! | `dmcl.download`     | [`download`]                |
//! | `dmcl.launch`       | [`launch`]                  |
//! | `dmcl.cli`          | [`cli`]                     |
//! | (no equivalent)     | [`mcjson`]                  |

// The crate is a library: a good part of its public surface is intentionally
// wider than what the CLI front-end calls (it is exposed for the future GUI).
// No blanket `#![allow(dead_code)]` here on purpose — `pub` items are not
// flagged anyway, so the lint still catches genuinely unused private helpers.

pub mod account;
pub mod cli;
pub mod config;
pub mod download;
pub mod env;
pub mod launch;
pub mod mcjson;
pub mod utils;
