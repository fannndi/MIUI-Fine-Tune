//! Host E2E for the MIUI bridge against a fake `settings` binary.
//!
//! Proves the full hold/restore cycle without a device: entering a mapped
//! game writes the perf mirror, entering a powersave-mapped app writes the
//! saver, and leaving restores the captured user values — all through the
//! same `settings` CLI plumbing the device uses.

mod common;

use common::{tmp, Daemon};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Fake `settings` script: one state file per `scope.key`.
fn write_fake_settings(dir: &Path) -> PathBuf {
    let state = dir.join("settings-state");
    std::fs::create_dir_all(&state).unwrap();
    let script = dir.join("settings");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\ns={}\ncase \"$1\" in\nget) cat \"$s/$2.$3\" 2>/dev/null ;;\nput) printf '%s' \"$4\" > \"$s/$2.$3\" ;;\nesac\n",
            state.display()
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(&script).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&script, perms).unwrap();
    state
}

fn setting(state: &Path, file: &str) -> Option<String> {
    std::fs::read_to_string(state.join(file))
        .ok()
        .map(|s| s.trim().to_string())
}

/// Polls a file for a needle (bridge persists happen right after events).
fn wait_file_contains(path: &Path, needle: &str, timeout: Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if let Ok(s) = std::fs::read_to_string(path) {
            if s.contains(needle) {
                return true;
            }
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn bridge_holds_and_restores_through_a_fake_settings_binary() {
    let dir = tmp("bridge");
    let state = write_fake_settings(&dir);
    std::fs::write(
        dir.join("config.json"),
        r#"{"schema":1,"enabled":true,"dynamic":true,"base_profile":"balance",
            "app_map":{"com.YoStarEN.AzurLane":"game","com.google.android.youtube":"powersave"}}"#,
    )
    .unwrap();

    let script = dir.join("settings");
    let mut d = Daemon::spawn_env(
        &dir.join("state"),
        &dir.join("config.json"),
        &[
            ("MIFINETUNE_SETTINGS_BIN", script.to_str().unwrap()),
            ("MIFINETUNE_LOGCAT_BIN", "/nonexistent-logcat"),
        ],
    );

    d.wait_for(|v| v["event"] == "hello", Duration::from_secs(5));
    d.send(json!({"cmd":"screen","on":true,"locked":false}));

    // --- mapped game -> perf mirror held at "high" ---------------------------
    // Note: the bridge sync fires before the (async, settled) apply; assert on
    // the fake settings state — the actual observable effect.
    d.send(json!({"cmd":"fg","pkg":"com.YoStarEN.AzurLane"}));
    d.wait_for(
        |v| v["event"] == "bridge" && v["msg"].as_str().unwrap_or("").contains("perf mirror ON"),
        Duration::from_secs(10),
    );
    assert_eq!(
        setting(&state, "system.power_mode").as_deref(),
        Some("high")
    );

    // --- leave the game -> restore the captured value ------------------------
    d.send(json!({"cmd":"fg","pkg":"com.miui.home"}));
    d.wait_for(
        |v| {
            v["event"] == "bridge"
                && v["msg"]
                    .as_str()
                    .unwrap_or("")
                    .contains("perf mirror restored")
        },
        Duration::from_secs(10),
    );
    assert_eq!(
        setting(&state, "system.power_mode").as_deref(),
        Some("middle")
    );

    // --- powersave-mapped app -> saver follow held at "1" ---------------------
    d.send(json!({"cmd":"fg","pkg":"com.google.android.youtube"}));
    d.wait_for(
        |v| v["event"] == "bridge" && v["msg"].as_str().unwrap_or("").contains("saver ON"),
        Duration::from_secs(10),
    );
    assert_eq!(setting(&state, "global.low_power").as_deref(), Some("1"));

    // --- leave -> saver restored to the user's own value ----------------------
    d.send(json!({"cmd":"fg","pkg":"com.miui.home"}));
    d.wait_for(
        |v| v["event"] == "bridge" && v["msg"].as_str().unwrap_or("").contains("saver restored"),
        Duration::from_secs(10),
    );
    assert_eq!(setting(&state, "global.low_power").as_deref(), Some("0"));

    // holds.json must be clean after the restore (no dangling hold); the
    // persist happens just after the event, so poll briefly.
    let holds_path = dir.join("state").join("holds.json");
    assert!(
        wait_file_contains(&holds_path, "\"perf_held\": false", Duration::from_secs(2)),
        "perf hold must be released in holds.json"
    );
    assert!(
        wait_file_contains(&holds_path, "\"saver_held\": false", Duration::from_secs(2)),
        "saver hold must be released in holds.json"
    );

    d.send(json!({"cmd":"shutdown"}));
    d.wait_for(|v| v["event"] == "bye", Duration::from_secs(5));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dynamic_off_stops_the_bridge_follow() {
    let dir = tmp("bridge-dynoff");
    let state = write_fake_settings(&dir);
    std::fs::write(
        dir.join("config.json"),
        r#"{"schema":1,"enabled":true,"dynamic":false,"base_profile":"balance",
            "app_map":{"com.YoStarEN.AzurLane":"game","com.google.android.youtube":"powersave"}}"#,
    )
    .unwrap();

    let script = dir.join("settings");
    let mut d = Daemon::spawn_env(
        &dir.join("state"),
        &dir.join("config.json"),
        &[
            ("MIFINETUNE_SETTINGS_BIN", script.to_str().unwrap()),
            ("MIFINETUNE_LOGCAT_BIN", "/nonexistent-logcat"),
        ],
    );

    d.wait_for(|v| v["event"] == "hello", Duration::from_secs(5));
    d.send(json!({"cmd":"screen","on":true,"locked":false}));
    d.send(json!({"cmd":"fg","pkg":"com.YoStarEN.AzurLane"}));
    // dynamic OFF: mapped game applies the base, no MIUI mode writes
    d.wait_for(
        |v| v["event"] == "applied" && v["profile"] == "balance",
        Duration::from_secs(10),
    );
    std::thread::sleep(Duration::from_millis(300)); // let any stray sync land
    assert_eq!(
        setting(&state, "system.power_mode"),
        None,
        "no write while dynamic is off"
    );
    assert_eq!(setting(&state, "global.low_power"), None);

    d.send(json!({"cmd":"shutdown"}));
    d.wait_for(|v| v["event"] == "bye", Duration::from_secs(5));
    let _ = std::fs::remove_dir_all(&dir);
}
