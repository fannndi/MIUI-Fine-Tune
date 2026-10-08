//! Live MIUI/Android flags read through the `settings` CLI.
//!
//! The daemon runs as root, so reads/writes go through `/system/bin/settings`
//! (binder-free, verified on device). Reads are polled on a background
//! thread and cached — the decision loop never blocks on exec.
//!
//! `MIFINETUNE_SETTINGS_BIN` overrides the binary path (host E2E tests and
//! simulations point it at a script); the `*_with` helpers take an explicit
//! path so tests never touch process globals.

use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// AOSP/MIUI battery saver global.
pub const SAVER_KEY: &str = "low_power";
/// MIUI performance switch mirror (the property itself is unreachable).
pub const POWER_MODE_KEY: &str = "power_mode";

const SETTINGS_BIN: &str = "/system/bin/settings";
/// Env override for the settings binary (host tests / simulation).
pub const SETTINGS_BIN_ENV: &str = "MIFINETUNE_SETTINGS_BIN";
const POLL_MS: u64 = 5_000;

/// Resolved settings binary for this process.
pub fn bin_path() -> String {
    std::env::var(SETTINGS_BIN_ENV).unwrap_or_else(|_| SETTINGS_BIN.to_string())
}

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
        let me = Arc::new(LiveFlags {
            saver: AtomicBool::new(false),
        });
        let keep = me.clone();
        let _ = std::thread::Builder::new()
            .name("live-flags".into())
            .spawn(move || loop {
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
    read_with(&bin_path(), scope, key)
}

/// Explicit-binary variant (tests; no process-global state).
pub fn read_with(bin: &str, scope: &str, key: &str) -> Option<String> {
    let out = Command::new(bin).args(["get", scope, key]).output().ok()?;
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

/// `settings put <scope> <key> <value>` (bridge writes).
pub fn put(scope: &str, key: &str, value: &str) -> Result<(), String> {
    put_with(&bin_path(), scope, key, value)
}

/// Explicit-binary variant (tests; no process-global state).
pub fn put_with(bin: &str, scope: &str, key: &str, value: &str) -> Result<(), String> {
    let out = Command::new(bin)
        .args(["put", scope, key, value])
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    /// Fake `settings` script: get/put against a state dir (one file per key).
    fn fake_settings(tag: &str) -> (PathBuf, PathBuf) {
        let dir = crate::daemon::test_util::tmpdir(tag);
        let state = dir.join("state");
        fs::create_dir_all(&state).unwrap();
        let script = dir.join("settings");
        fs::write(
            &script,
            format!(
                "#!/bin/sh\ns={}\ncase \"$1\" in\nget) cat \"$s/$2.$3\" 2>/dev/null ;;\nput) printf '%s' \"$4\" > \"$s/$2.$3\" ;;\nesac\n",
                state.display()
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perm = fs::metadata(&script).unwrap().permissions();
            perm.set_mode(0o755);
            fs::set_permissions(&script, perm).unwrap();
        }
        (script, state)
    }

    #[test]
    fn read_put_roundtrip_through_a_fake_binary() {
        let (script, _state) = fake_settings("settings-rt");
        let bin = script.to_str().unwrap();

        // unset value -> None (script exits 0 with empty output)
        assert_eq!(read_with(bin, "system", "power_mode"), None);

        put_with(bin, "system", "power_mode", "high").unwrap();
        assert_eq!(
            read_with(bin, "system", "power_mode").as_deref(),
            Some("high")
        );

        put_with(bin, "system", "power_mode", "middle").unwrap();
        assert_eq!(
            read_with(bin, "system", "power_mode").as_deref(),
            Some("middle")
        );

        put_with(bin, "global", "low_power", "1").unwrap();
        assert_eq!(read_with(bin, "global", "low_power").as_deref(), Some("1"));
    }

    #[test]
    fn missing_binary_is_none_not_panic() {
        assert_eq!(
            read_with("/nonexistent/settings-bin", "system", "power_mode"),
            None
        );
        assert!(put_with("/nonexistent/settings-bin", "system", "power_mode", "high").is_err());
    }
}
