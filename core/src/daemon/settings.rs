//! Live MIUI/Android flags read through the `settings` CLI.
//!
//! The daemon runs as root, so reads/writes go through `/system/bin/settings`
//! (binder-free, verified on device). Reads are polled on a background
//! thread and cached — the decision loop never blocks on exec.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// AOSP/MIUI battery saver global.
pub const SAVER_KEY: &str = "low_power";
/// MIUI performance switch mirror (the property itself is unreachable).
pub const POWER_MODE_KEY: &str = "power_mode";

const SETTINGS_BIN: &str = "/system/bin/settings";
const POLL_MS: u64 = 5_000;

/// Cached live flags for the decision loop.
pub struct LiveFlags {
    saver: AtomicBool,
}

impl LiveFlags {
    /// Last polled battery-saver value (user + framework, unattributed).
    pub fn saver(&self) -> bool {
        self.saver.load(Ordering::Relaxed)
    }

    /// Starts the polling thread; lives for the daemon lifetime.
    pub fn spawn() -> Arc<Self> {
        let me = Arc::new(LiveFlags { saver: AtomicBool::new(false) });
        let keep = me.clone();
        let _ = std::thread::Builder::new().name("live-flags".into()).spawn(move || loop {
            if let Some(v) = read_bool("global", SAVER_KEY) {
                keep.saver.store(v, Ordering::Relaxed);
            }
            std::thread::sleep(Duration::from_millis(POLL_MS));
        });
        me
    }
}

/// `settings get <scope> <key>`; None when the binary or value is missing.
pub fn read(scope: &str, key: &str) -> Option<String> {
    let out = std::process::Command::new(SETTINGS_BIN)
        .args(["get", scope, key])
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() || s == "null" {
        None
    } else {
        Some(s)
    }
}

pub fn read_bool(scope: &str, key: &str) -> Option<bool> {
    read(scope, key).map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
}

/// `settings put <scope> <key> <value>` (bridge writes, F4).
pub fn put(scope: &str, key: &str, value: &str) -> Result<(), String> {
    let out = std::process::Command::new(SETTINGS_BIN)
        .args(["put", scope, key, value])
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}
