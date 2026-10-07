//! MiFineTune core — MIUI-harmonized profile switcher.
//!
//! Architecture: this crate is the single writer of tuning parameters.
//! Kotlin (UI) only shells out to the `miui-ft` binary and renders reports.
//! Ownership model (see `catalog`): FREE + BOOT-BASELINE parameters only;
//! runtime-framework nodes are rejected even if a profile names them.

pub mod apply;
pub mod catalog;
pub mod probe;
pub mod profile;

/// Default profile set, bundled in the binary and mirrored in assets.
pub const DEFAULT_PROFILES_JSON: &str = include_str!("../profiles.json");
