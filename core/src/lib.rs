//! MiFineTune core — MIUI-harmonized profile switcher.
//!
//! Architecture: this crate is the single writer of tuning parameters.
//! The Kotlin app drives it two ways: one-shot CLI commands (`plan`, `apply`,
//! ...) and the `serve` stdio daemon (JSON-lines) used by the service.
//!
//! Module index:
//! - `engine/`  planning + apply/verify/restore (see engine/mod.rs)
//! - `serve.rs` daemon entry (stdio JSON-lines)
//!
//! Ownership model (see `engine::catalog`): FREE + BOOT-BASELINE parameters
//! only; runtime-framework nodes are rejected even if a profile names them.

pub mod engine;
pub mod serve;

/// Default profile set, bundled in the binary and mirrored in assets.
pub const DEFAULT_PROFILES_JSON: &str = include_str!("../profiles.json");
