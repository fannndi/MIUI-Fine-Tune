//! Host E2E for the daemon's logcat watchers, using a fake `logcat` script.
//!
//! Exercises the real spawn/parse/freshness pipeline without a device:
//! - foreground stream: initial package, then a live switch on demand
//! - multi-window stream: off at start, ON when the marker file appears
//! - peek (`-d`): seeds wake/unlock decisions
//!
//! The fake script emits lines with the CURRENT epoch (`date +%s`), which is
//! exactly what `logcat -v epoch` does on device.

mod common;

use common::{tmp, Daemon};
use serde_json::json;
use std::time::Duration;

fn write_fake_logcat(dir: &std::path::Path) {
    let script = r#"#!/bin/sh
EPOCH=$(date +%s)
case "$*" in
  *"-b main"*)
    echo "$EPOCH.000  1234  1234 D GameBoosterService: onGameStatusChange id=1 mForegroundPackageName='com.miui.home' mMultiWindowForegroundPackageName='null'"
    i=0
    while [ $i -lt 240 ]; do
      if [ -f "$MIFINETUNE_MW_FILE" ]; then
        echo "$EPOCH.000  1234  1234 D GameBoosterService: onGameStatusChange id=1 mForegroundPackageName='com.miui.home' mMultiWindowForegroundPackageName='com.android.chrome'"
        rm -f "$MIFINETUNE_MW_FILE"
      fi
      if [ -f "$MIFINETUNE_MW_OFF_FILE" ]; then
        echo "$EPOCH.000  1234  1234 D GameBoosterService: onGameStatusChange id=1 mForegroundPackageName='com.miui.home' mMultiWindowForegroundPackageName='null'"
        rm -f "$MIFINETUNE_MW_OFF_FILE"
      fi
      if [ -f "$MIFINETUNE_JANK_FILE" ]; then
        echo "$EPOCH.000  1234  1234 I Choreographer: Skipped 42 frames!  The application may be doing too much work on its main thread."
        rm -f "$MIFINETUNE_JANK_FILE"
      fi
      sleep 0.5
      i=$((i+1))
    done
    ;;
  *"-d"*)
    echo "$EPOCH.000  1234  1234 I am_resume_activity: [0,1,2,com.YoStarEN.AzurLane/.Main,3]"
    ;;
  *)
    echo "$EPOCH.000  1234  1234 I am_resume_activity: [0,1,2,com.whatsapp/.Main,3]"
    i=0
    while [ $i -lt 240 ]; do
      if [ -f "$MIFINETUNE_FG_FILE" ]; then
        echo "$EPOCH.000  1234  1234 I am_resume_activity: [0,1,2,com.google.android.youtube/.Main,3]"
        rm -f "$MIFINETUNE_FG_FILE"
      fi
      sleep 0.5
      i=$((i+1))
    done
    ;;
esac
"#;
    let path = dir.join("fake-logcat.sh");
    std::fs::write(&path, script).unwrap();
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&path, perms).unwrap();
}

#[test]
fn watchers_drive_the_full_pipeline() {
    let dir = tmp("e2e");
    write_fake_logcat(&dir);
    std::fs::write(
        dir.join("config.json"),
        r#"{"schema":1,"enabled":true,"dynamic":true,"base_profile":"balance",
            "app_map":{"com.YoStarEN.AzurLane":"game","com.google.android.youtube":"powersave"}}"#,
    )
    .unwrap();

    let logcat = dir.join("fake-logcat.sh");
    let fg = dir.join("fg-on");
    let mw = dir.join("mw-on");
    let mw_off = dir.join("mw-off");
    let mut d = Daemon::spawn_env(
        &dir.join("state"),
        &dir.join("config.json"),
        &[
            ("MIFINETUNE_LOGCAT_BIN", logcat.to_str().unwrap()),
            ("MIFINETUNE_FG_FILE", fg.to_str().unwrap()),
            ("MIFINETUNE_MW_FILE", mw.to_str().unwrap()),
            ("MIFINETUNE_MW_OFF_FILE", mw_off.to_str().unwrap()),
        ],
    );

    // The fake foreground stream starts with whatsapp -> base apply.
    d.send(json!({"cmd":"hello"}));
    let applied = d.wait_for(
        |v| v["event"] == "applied" && v["profile"] == "balance",
        Duration::from_secs(10),
    );
    assert_eq!(applied["reason"], "base");

    // Wake seed: screen on/unlocked -> peek (-d) returns Azur Lane -> game.
    d.send(json!({"cmd":"screen","on":true,"locked":false}));
    let applied = d.wait_for(
        |v| v["event"] == "applied" && v["profile"] == "game",
        Duration::from_secs(10),
    );
    assert_eq!(applied["reason"], "app");
    assert_eq!(applied["src_pkg"], "com.YoStarEN.AzurLane");

    // Multi-window ON via the fake main-buffer stream -> balance forced.
    std::fs::write(dir.join("mw-on"), "1").unwrap();
    let applied = d.wait_for(
        |v| v["event"] == "applied" && v["profile"] == "balance" && v["reason"] == "multi-window",
        Duration::from_secs(10),
    );
    assert_eq!(applied["reason"], "multi-window");
    let st = d.wait_for(
        |v| v["event"] == "state" && v["state"]["multi_window"] == true,
        Duration::from_secs(5),
    );
    assert_eq!(st["state"]["second_window"], "com.android.chrome");

    // Multi-window OFF -> back to the mapped game (Azur Lane still in front).
    std::fs::write(dir.join("mw-off"), "1").unwrap();
    let applied = d.wait_for(
        |v| v["event"] == "applied" && v["profile"] == "game" && v["reason"] == "app",
        Duration::from_secs(10),
    );
    assert_eq!(applied["src_pkg"], "com.YoStarEN.AzurLane");

    // Live foreground switch to a powersave-mapped app.
    std::fs::write(dir.join("fg-on"), "1").unwrap();
    let applied = d.wait_for(
        |v| v["event"] == "applied" && v["profile"] == "powersave",
        Duration::from_secs(10),
    );
    assert_eq!(applied["reason"], "app");
    assert_eq!(applied["src_pkg"], "com.google.android.youtube");

    d.send(json!({"cmd":"shutdown"}));
    d.wait_for(|v| v["event"] == "bye", Duration::from_secs(5));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn jank_boost_overlay_round_trip() {
    let dir = tmp("jank");
    write_fake_logcat(&dir);
    std::fs::write(
        dir.join("config.json"),
        r#"{"schema":1,"enabled":true,"dynamic":true,"base_profile":"balance","jank_boost":true}"#,
    )
    .unwrap();

    let logcat = dir.join("fake-logcat.sh");
    let fg = dir.join("fg-on");
    let mw = dir.join("mw-on");
    let mw_off = dir.join("mw-off");
    let jank = dir.join("jank-on");
    let mut d = Daemon::spawn_env(
        &dir.join("state"),
        &dir.join("config.json"),
        &[
            ("MIFINETUNE_LOGCAT_BIN", logcat.to_str().unwrap()),
            ("MIFINETUNE_FG_FILE", fg.to_str().unwrap()),
            ("MIFINETUNE_MW_FILE", mw.to_str().unwrap()),
            ("MIFINETUNE_MW_OFF_FILE", mw_off.to_str().unwrap()),
            ("MIFINETUNE_JANK_FILE", jank.to_str().unwrap()),
            ("MIFINETUNE_BOOST_SECS", "1"),
            ("MIFINETUNE_BOOST_COOLDOWN_SECS", "0"),
        ],
    );

    // base decision first (the fake fg stream seeds whatsapp)
    d.wait_for(
        |v| v["event"] == "applied" && v["profile"] == "balance",
        Duration::from_secs(10),
    );

    // a jank burst raises the hidden boost overlay
    std::fs::write(&jank, "1").unwrap();
    let applied = d.wait_for(
        |v| v["event"] == "applied" && v["profile"] == "boost",
        Duration::from_secs(10),
    );
    assert_eq!(applied["reason"], "jank");

    // after the short window the normal decision returns
    let applied = d.wait_for(
        |v| v["event"] == "applied" && v["profile"] == "balance",
        Duration::from_secs(10),
    );
    assert_eq!(applied["ok"], true);

    d.send(json!({"cmd":"shutdown"}));
    d.wait_for(|v| v["event"] == "bye", Duration::from_secs(5));
    let _ = std::fs::remove_dir_all(&dir);
}
