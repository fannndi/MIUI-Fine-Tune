//! Engine — the tuning core. Everything that decides or touches parameters.
//!
//! File index:
//! - `catalog/`    writable parameter registry + forbidden-path guard
//! - `probe.rs`    read-only device capture (values + options + evidence)
//! - `profile.rs`  profiles.json model + parser
//! - `plan.rs`     profile + probe -> ordered Plan (statuses, invariants)
//! - `validate.rs` per-kind value validation (clamps to real device options)
//! - `readback.rs` read-back comparison rules (harmony: framework wins)
//! - `apply/`      snapshot -> guarded writes -> verify; restore; drift
//! - `testutil.rs` shared test fixtures (cfg(test) only)
//!
//! Invariant: every write path passes `catalog::guard_path` — framework-owned
//! nodes are rejected even if a profile names them. See docs/ROM-HARMONY.md.

pub mod apply;
pub mod catalog;
pub mod doctor;
pub mod plan;
pub mod profile;
pub mod probe;
pub mod readback;
pub mod validate;

#[cfg(test)]
pub(crate) mod testutil;
