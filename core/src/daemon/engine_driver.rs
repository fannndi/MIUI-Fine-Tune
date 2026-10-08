//! `EngineDriver` implementation: the in-process engine.
//!
//! The daemon IS the engine (same binary), so apply/restore are direct
//! function calls — no `su` round-trips, no CLI parsing.

use super::worker::{EngineDriver, Outcome};
use crate::engine::apply::{self, Store};
use crate::engine::probe;
use std::path::Path;

pub struct EngineApplier {
    store: Store,
}

impl EngineApplier {
    pub fn new(state_dir: &Path) -> Self {
        EngineApplier { store: Store::new(state_dir) }
    }
}

impl EngineDriver for EngineApplier {
    fn active(&mut self) -> Option<String> {
        self.store.load_state().active
    }

    fn apply(&mut self, profile_id: &str) -> Outcome {
        let files = match self.store.load_profiles(None) {
            Ok(f) => f,
            Err(e) => return Outcome::err(e),
        };
        let Some(profile) = files.profiles.iter().find(|p| p.id == profile_id) else {
            return Outcome::err(format!("unknown profile '{profile_id}'"));
        };
        match apply::apply_with_pass2(&self.store, profile) {
            Ok(rep) => Outcome {
                ok: rep.ok,
                wrote: rep.wrote,
                failed: rep.failed,
                error: None,
                already: false,
            },
            Err(e) => Outcome::err(e),
        }
    }

    fn restore(&mut self) -> Outcome {
        // Restore is idempotent: nothing to restore is a clean success
        // (the service-off flow must never fail on an empty snapshot).
        if self.store.load_snapshot().is_none() {
            return Outcome { ok: true, wrote: 0, failed: 0, error: None, already: false };
        }
        match apply::restore(&self.store, &probe::probe()) {
            Ok(rep) => Outcome {
                ok: rep.ok,
                wrote: rep.wrote,
                failed: rep.failed,
                error: None,
                already: false,
            },
            Err(e) => Outcome::err(e),
        }
    }
}
