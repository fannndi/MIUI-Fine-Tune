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

fn chmod(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).unwrap();
}

/// Fake `settings` script: one state file per `scope.key`.
fn write_fake_settings(dir: &Path) -> PathBuf {
    let state = dir.join("settings-state");
    std::fs::create_dir_all(&state).unwrap();
    let script = dir.join("settings");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\ns={}\ncase \"$1\" in\nget) cat \"$s/$2.$3\" 2>/dev/null ;;\nput) printf '%s' \"$4\" > \"$s/$2.$3\" ;;\ndelete) rm -f \"$s/$2.$3\" ;;\nesac\n",
            state.display()
        ),
    )
    .unwrap();
    chmod(&script);
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
fn charge_guard_pauses_and_resumes_with_hysteresis() {
    let dir = tmp("bridge-charge");
    // fake sysfs: charging at 90%, charge node currently enabled
    let root = dir.join("fake-root");
    let w = |rel: &str, body: &str| {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    };
    w("sys/class/power_supply/battery/capacity", "90\n");
    w("sys/class/power_supply/battery/status", "Charging\n");
    w(
        "sys/class/power_supply/battery/battery_charging_enabled",
        "1\n",
    );

    std::fs::write(
        dir.join("config.json"),
        r#"{"schema":1,"enabled":true,"dynamic":true,"base_profile":"balance",
            "charge_limit":true,"charge_limit_pct":80}"#,
    )
    .unwrap();

    let mut d = Daemon::spawn_env(
        &dir.join("state"),
        &dir.join("config.json"),
        &[
            ("MIFINETUNE_SYSFS_ROOT", root.to_str().unwrap()),
            ("MIFINETUNE_ENV_SAMPLE_MS", "200"),
            ("MIFINETUNE_LOGCAT_BIN", "/nonexistent-logcat"),
        ],
    );

    let node = root.join("sys/class/power_supply/battery/battery_charging_enabled");
    d.wait_for(|v| v["event"] == "hello", Duration::from_secs(5));
    d.wait_for(
        |v| v["event"] == "env" && v["env"]["battery_pct"] == 90,
        Duration::from_secs(5),
    );

    // at/above the limit while charging -> pause (node 0)
    d.wait_for(
        |v| v["event"] == "bridge" && v["msg"].as_str().unwrap_or("").contains("charge paused"),
        Duration::from_secs(10),
    );
    assert_eq!(std::fs::read_to_string(&node).unwrap().trim(), "0");

    // the charger now reports "not charging" because of our pause: the held
    // state must keep it paused (no oscillation)
    w("sys/class/power_supply/battery/status", "Not charging\n");
    std::thread::sleep(Duration::from_millis(700));
    assert_eq!(std::fs::read_to_string(&node).unwrap().trim(), "0");

    // battery drains below the hysteresis floor -> resume
    w("sys/class/power_supply/battery/capacity", "70\n");
    d.wait_for(
        |v| v["event"] == "bridge" && v["msg"].as_str().unwrap_or("").contains("charge resumed"),
        Duration::from_secs(10),
    );
    assert_eq!(std::fs::read_to_string(&node).unwrap().trim(), "1");

    // the hold is released in holds.json
    let holds_path = dir.join("state").join("holds.json");
    assert!(
        wait_file_contains(
            &holds_path,
            "\"charge_held\": false",
            Duration::from_secs(2)
        ),
        "charge hold must be released"
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

/// Apps Profile software layer E2E: a mapped app with bypass + DND engages
/// both through the real daemon code paths (fake sysfs + fake settings), the
/// floor releases bypass, and leaving the app releases DND.
#[test]
fn app_profile_software_engages_and_restores() {
    let dir = tmp("bridge-appprofile");
    let _state = write_fake_settings(&dir);
    let root = dir.join("fake-root");
    let w = |rel: &str, body: &str| {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    };
    w("sys/class/power_supply/battery/capacity", "80\n");
    w("sys/class/power_supply/usb/online", "1\n");
    w("sys/class/power_supply/battery/input_suspend", "0\n");

    std::fs::write(
        dir.join("config.json"),
        r#"{"schema":1,"enabled":true,"dynamic":true,"base_profile":"balance",
            "bypass_floor_pct":30,
            "app_profiles":{"com.g":{"profile":"game","bypass_charge":true,
                                     "dnd":"priority"}}}"#,
    )
    .unwrap();

    let mut d = Daemon::spawn_env(
        &dir.join("state"),
        &dir.join("config.json"),
        &[
            (
                "MIFINETUNE_SETTINGS_BIN",
                dir.join("settings").to_str().unwrap(),
            ),
            ("MIFINETUNE_LOGCAT_BIN", "/nonexistent-logcat"),
            ("MIFINETUNE_SYSFS_ROOT", root.to_str().unwrap()),
        ],
    );

    let node = root.join("sys/class/power_supply/battery/input_suspend");
    d.wait_for(|v| v["event"] == "hello", Duration::from_secs(5));
    d.send(json!({"cmd":"dnd_access","granted":true}));
    d.send(json!({"cmd":"screen","on":true,"locked":false}));
    d.send(json!({"cmd":"fg","pkg":"com.g"}));

    // bypass engages (node 1) while the charger is online
    d.wait_for(
        |v| {
            v["event"] == "bridge"
                && v["msg"]
                    .as_str()
                    .unwrap_or("")
                    .contains("bypass charging ON")
        },
        Duration::from_secs(10),
    );
    assert_eq!(std::fs::read_to_string(&node).unwrap().trim(), "1");

    // DND decision emitted for the app (executed by the app on device)
    let dnd = d.wait_for(
        |v| v["event"] == "dnd" && v.get("mode").is_some(),
        Duration::from_secs(10),
    );
    assert_eq!(dnd["mode"], "priority");

    // battery drops below the floor -> bypass releases (charging returns)
    w("sys/class/power_supply/battery/capacity", "25\n");
    d.send(json!({"cmd":"dnd_access","granted":true})); // trigger one more sync
    d.wait_for(
        |v| {
            v["event"] == "bridge"
                && v["msg"]
                    .as_str()
                    .unwrap_or("")
                    .contains("bypass charging OFF")
        },
        Duration::from_secs(10),
    );
    assert_eq!(std::fs::read_to_string(&node).unwrap().trim(), "0");

    // leaving the app releases DND
    d.send(json!({"cmd":"fg","pkg":"com.miui.home"}));
    let dnd = d.wait_for(|v| v["event"] == "dnd", Duration::from_secs(10));
    assert!(
        dnd.get("mode").is_none(),
        "release event carries no mode: {dnd}"
    );

    // holds are clean after the release
    let holds_path = dir.join("state").join("holds.json");
    assert!(
        wait_file_contains(
            &holds_path,
            "\"bypass_held\": false",
            Duration::from_secs(2)
        ),
        "bypass hold must be released"
    );

    d.send(json!({"cmd":"shutdown"}));
    d.wait_for(|v| v["event"] == "bye", Duration::from_secs(5));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn refresh_follow_per_app_and_sleep() {
    let dir = tmp("bridge-refresh");
    let state = write_fake_settings(&dir);
    // the user's own value: 120 Hz
    std::fs::write(state.join("system.user_refresh_rate"), "120").unwrap();
    std::fs::write(
        dir.join("config.json"),
        r#"{"schema":1,"enabled":true,"dynamic":true,"base_profile":"balance",
            "app_profiles":{
              "com.miui.home":{"refresh_hz":90},
              "com.google.android.youtube":{"refresh_hz":60}}}"#,
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

    // launcher entry (90) captured the user's 120, wrote 90
    d.send(json!({"cmd":"fg","pkg":"com.miui.home"}));
    d.wait_for(
        |v| {
            v["event"] == "bridge"
                && v["msg"]
                    .as_str()
                    .unwrap_or("")
                    .contains("refresh follow 90 Hz (com.miui.home)")
        },
        Duration::from_secs(10),
    );
    assert_eq!(
        setting(&state, "system.user_refresh_rate").as_deref(),
        Some("90")
    );

    // video app entry -> 60 (capture must NOT move)
    d.send(json!({"cmd":"fg","pkg":"com.google.android.youtube"}));
    d.wait_for(
        |v| {
            v["event"] == "bridge"
                && v["msg"]
                    .as_str()
                    .unwrap_or("")
                    .contains("refresh follow 60 Hz (com.google.android.youtube)")
        },
        Duration::from_secs(10),
    );
    assert_eq!(
        setting(&state, "system.user_refresh_rate").as_deref(),
        Some("60")
    );

    // an app without a target releases -> the captured 120 returns
    d.send(json!({"cmd":"fg","pkg":"com.whatsapp"}));
    d.wait_for(
        |v| {
            v["event"] == "bridge"
                && v["msg"]
                    .as_str()
                    .unwrap_or("")
                    .contains("refresh restored (120)")
        },
        Duration::from_secs(10),
    );
    assert_eq!(
        setting(&state, "system.user_refresh_rate").as_deref(),
        Some("120")
    );

    // screen off -> the fixed sleep rate (30 Hz). The sleep decision fires
    // after the 10 s grace (SLEEP_DELAY_MS), so this wait is longer.
    d.send(json!({"cmd":"screen","on":false,"locked":true}));
    d.wait_for(
        |v| {
            v["event"] == "bridge"
                && v["msg"]
                    .as_str()
                    .unwrap_or("")
                    .contains("refresh follow 30 Hz (sleep)")
        },
        Duration::from_secs(25),
    );
    assert_eq!(
        setting(&state, "system.user_refresh_rate").as_deref(),
        Some("30")
    );

    // wake -> release again (no app target for whatsapp)
    d.send(json!({"cmd":"screen","on":true,"locked":false}));
    d.wait_for(
        |v| {
            v["event"] == "bridge"
                && v["msg"]
                    .as_str()
                    .unwrap_or("")
                    .contains("refresh restored (120)")
        },
        Duration::from_secs(10),
    );
    assert_eq!(
        setting(&state, "system.user_refresh_rate").as_deref(),
        Some("120")
    );

    // holds are clean after the release
    let holds_path = dir.join("state").join("holds.json");
    assert!(
        wait_file_contains(
            &holds_path,
            "\"refresh_held\": false",
            Duration::from_secs(2)
        ),
        "refresh hold must be released"
    );

    d.send(json!({"cmd":"shutdown"}));
    d.wait_for(|v| v["event"] == "bye", Duration::from_secs(5));
    let _ = std::fs::remove_dir_all(&dir);
}
