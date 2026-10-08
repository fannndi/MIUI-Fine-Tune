//! Environment sampler thread: read-only telemetry into the main loop.
//!
//! The sampler itself lives in `engine::env` (shared with `doctor`); this
//! file only owns the cadence + channel plumbing. The thread exits when the
//! daemon's message channel closes (process shutdown).

use super::Msg;
use crate::engine::env::{self, EnvSnapshot, Sampler};
use std::sync::mpsc::Sender;
use std::time::Duration;

/// Sampling cadence: battery/thermal are slow signals; 30 s keeps the
/// telemetry fresh without waking up the device.
pub const ENV_SAMPLE_MS: u64 = 30_000;

/// Starts the sampler thread (first sample immediately, then cadence).
pub fn spawn(tx: Sender<Msg>) {
    let sampler = Sampler::new(&env::default_root());
    let _ = std::thread::Builder::new().name("env".into()).spawn(move || loop {
        let snap: EnvSnapshot = sampler.sample();
        if tx.send(Msg::Env(snap)).is_err() {
            break; // main loop gone — process is shutting down
        }
        std::thread::sleep(Duration::from_millis(ENV_SAMPLE_MS));
    });
}
