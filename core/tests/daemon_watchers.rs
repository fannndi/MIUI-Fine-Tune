//! Host E2E for the daemon's logcat watchers, using a fake `logcat` script.
//!
//! Exercises the real spawn/parse/freshness pipeline without a device:
//! - foreground stream: initial package, then a live switch on demand
//! - multi-window stream: off at start, ON when the marker file appears
//! - peek (`-d`): seeds wake/unlock decisions
//!
//! The fake script emits lines with the CURRENT epoch (`date +%s`), which is
//! exactly what `logcat -v epoch` does on device.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

struct Daemon {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Daemon {
    fn spawn(dir: &std::path::Path) -> Self {
        let bin = env!("CARGO_BIN_EXE_miui-ft");
        let mut child = Command::new(bin)
            .args([
                "serve",
                "--state-dir",
                dir.join("state").to_str().unwrap(),
                "--config",
                dir.join("config.json").to_str().unwrap(),
            ])
            .env("MIFINETUNE_LOGCAT_BIN", dir.join("fake-logcat.sh"))
            .env("MIFINETUNE_FG_FILE", dir.join("fg-on"))
            .env("MIFINETUNE_MW_FILE", dir.join("mw-on"))
            .env("MIFINETUNE_MW_OFF_FILE", dir.join("mw-off"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn daemon");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Daemon { child, stdin, stdout }
    }

    fn send(&mut self, v: Value) {
        writeln!(self.stdin, "{v}").unwrap();
        self.stdin.flush().unwrap();
    }

    fn wait_for(&mut self, pred: impl Fn(&Value) -> bool, timeout: Duration) -> Value {
        let deadline = Instant::now() + timeout;
        let mut line = String::new();
        loop {
            assert!(Instant::now() < deadline, "timeout; last: {line}");
            line.clear();
            if self.stdout.read_line(&mut line).unwrap_or(0) == 0 {
                panic!("daemon stdout closed");
            }
            let v: Value = serde_json::from_str(line.trim()).expect("json line");
            if pred(&v) {
                return v;
            }
        }
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = writeln!(self.stdin, "{}", json!({"cmd":"shutdown"}));
        let _ = self.stdin.flush();
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if let Ok(Some(_)) = self.child.try_wait() {
                return;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn tmp(tag: &str) -> std::path::PathBuf {
    let mut d = std::env::temp_dir();
    d.push(format!("mifinetune-watch-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

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

    let mut d = Daemon::spawn(&dir);

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
