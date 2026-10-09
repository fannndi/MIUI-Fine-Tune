//! `EngineDriver` implementation: the in-process engine.
//!
//! The daemon IS the engine (same binary), so apply/restore are direct
//! function calls — no `su` round-trips, no CLI parsing.

use super::bridge::Bridge;
use super::worker::{EngineDriver, Outcome};
use crate::engine::apply::{self, Store};
use std::path::Path;
use std::sync::Arc;

pub struct EngineApplier {
    store: Store,
    bridge: Arc<Bridge>,
}

impl EngineApplier {
    pub fn new(state_dir: &Path, bridge: Arc<Bridge>) -> Self {
        EngineApplier {
            store: Store::new(state_dir),
            bridge,
        }
    }
}

impl EngineDriver for EngineApplier {
    fn active(&mut self) -> Option<String> {
        self.store.load_state().active
    }

    fn apply(&mut self, profile_id: &str, reconcile: bool) -> Outcome {
        let files = match self.store.load_profiles(None) {
            Ok(f) => f,
            Err(e) => return Outcome::err(e),
        };
        let Some(profile) = files.profiles.iter().find(|p| p.id == profile_id) else {
            return Outcome::err(format!("unknown profile '{profile_id}'"));
        };
        match apply::apply_with_pass2(&self.store, profile, reconcile) {
            Ok(rep) => Outcome {
                ok: rep.ok,
                wrote: rep.wrote,
                verified: rep.verified,
                failed: rep.failed,
                error: None,
                already: false,
            },
            Err(e) => Outcome::err(e),
        }
    }

    fn restore(&mut self) -> Outcome {
        // v0.11 service-off semantics: hands-off, zero interference.
        // Release every bridge artifact we hold (MIUI perf/saver mirror,
        // charge limit, bypass, DND) but never re-write catalog nodes:
        // the user asked for "matikan service = lepas intervensi", so the
        // device state is left to MIUI's own controllers. The stock values
        // stay on disk for the reconcile pass; the explicit `miui-ft
        // restore` CLI remains the repair path.
        self.bridge.release_all();
        let mut st = self.store.load_state();
        st.active = None;
        st.updated = apply::now_secs();
        st.last_mode = "hands-off".into();
        let _ = self.store.save_state(&st);
        Outcome {
            ok: true,
            wrote: 0,
            verified: 0,
            failed: 0,
            error: None,
            already: false,
        }
    }

    fn release_holds(&mut self) {
        self.bridge.release_all();
    }
}
