//! End-to-end daemon smoke test on the host: spawns the real `miui-ft
//! serve` binary, exchanges JSON-lines, asserts the decision pipeline.
//!
//! Runs against fake sysfs (host): every engine op reports "node missing"
//! (locked), which is a clean apply — perfect for exercising the protocol,
//! arbiter, worker and config path without a device.

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
    fn spawn(state_dir: &std::path::Path, config: &std::path::Path) -> Self {
        Self::spawn_env(state_dir, config, &[])
    }

    fn spawn_env(state_dir: &std::path::Path, config: &std::path::Path, envs: &[(&str, &str)]) -> Self {
        let bin = env!("CARGO_BIN_EXE_miui-ft");
        let mut cmd = Command::new(bin);
        cmd.args([
            "serve",
            "--state-dir",
            state_dir.to_str().unwrap(),
            "--config",
            config.to_str().unwrap(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
        for (k, v) in envs {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().expect("spawn daemon");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Daemon { child, stdin, stdout }
    }

    fn send(&mut self, v: Value) {
        writeln!(self.stdin, "{v}").unwrap();
        self.stdin.flush().unwrap();
    }

    /// Reads events until `pred` matches (or timeout). Returns the event.
    fn wait_for(&mut self, pred: impl Fn(&Value) -> bool, timeout: Duration) -> Value {
        let deadline = Instant::now() + timeout;
        let mut line = String::new();
        loop {
            assert!(
                Instant::now() < deadline,
                "timeout waiting for event; last line: {line}"
            );
            line.clear();
            if self.stdout.read_line(&mut line).unwrap_or(0) == 0 {
                panic!("daemon stdout closed unexpectedly");
            }
            let v: Value = serde_json::from_str(line.trim()).expect("daemon line is JSON");
            if pred(&v) {
                return v;
            }
        }
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.stdin.write_all(b"{\"cmd\":\"shutdown\"}\n");
        let _ = self.stdin.flush();
        let _ = self.child.wait_timeout(Duration::from_secs(3));
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

trait WaitTimeout {
    fn wait_timeout(&mut self, d: Duration) -> std::io::Result<Option<std::process::ExitStatus>>;
}
impl WaitTimeout for Child {
    fn wait_timeout(&mut self, d: Duration) -> std::io::Result<Option<std::process::ExitStatus>> {
        let deadline = Instant::now() + d;
        loop {
            if let Some(st) = self.try_wait()? {
                return Ok(Some(st));
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }
}

fn tmp(tag: &str) -> std::path::PathBuf {
    let mut d = std::env::temp_dir();
    d.push(format!("mifinetune-smoke-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn daemon_smoke_full_decision_path() {
    let dir = tmp("daemon");
    let cfg_path = dir.join("config.json");
    std::fs::write(
        &cfg_path,
        r#"{"schema":1,"enabled":true,"dynamic":true,"base_profile":"balance",
            "app_map":{"com.YoStarEN.AzurLane":"game","com.google.android.youtube":"powersave"}}"#,
    )
    .unwrap();

    let mut d = Daemon::spawn(&dir, &cfg_path);

    // 1. hello handshake -> hello + initial state
    d.send(json!({"cmd":"hello"}));
    let hello = d.wait_for(|v| v["event"] == "hello", Duration::from_secs(5));
    assert_eq!(hello["version"], 1);

    // 2. screen on/unlocked + foreground whatsapp -> applies base (balance)
    d.send(json!({"cmd":"screen","on":true,"locked":false}));
    d.send(json!({"cmd":"fg","pkg":"com.whatsapp"}));
    let applied = d.wait_for(
        |v| v["event"] == "applied" && v["profile"] == "balance",
        Duration::from_secs(10),
    );
    assert_eq!(applied["reason"], "base");
    assert_eq!(applied["ok"], true);

    // 3. mapped app in front -> game
    d.send(json!({"cmd":"fg","pkg":"com.YoStarEN.AzurLane"}));
    let applied = d.wait_for(
        |v| v["event"] == "applied" && v["profile"] == "game",
        Duration::from_secs(10),
    );
    assert_eq!(applied["reason"], "app");
    assert_eq!(applied["src_pkg"], "com.YoStarEN.AzurLane");

    // 4. state snapshot reflects active profile
    let st = d.wait_for(
        |v| v["event"] == "state" && v["state"]["active"] == "game",
        Duration::from_secs(5),
    );
    assert_eq!(st["state"]["foreground"], "com.YoStarEN.AzurLane");

    // 5. multi-window -> balance (multi-window reason)
    d.send(json!({"cmd":"mw","active":true,"other":"com.android.chrome"}));
    let applied = d.wait_for(
        |v| v["event"] == "applied" && v["profile"] == "balance",
        Duration::from_secs(10),
    );
    assert_eq!(applied["reason"], "multi-window");
    d.send(json!({"cmd":"mw","active":false,"other":null}));
    d.wait_for(|v| v["event"] == "applied" && v["profile"] == "game", Duration::from_secs(10));

    // 6. dynamic off -> mapped app falls back to base
    //    (app writes config.json first, then pings config_changed)
    std::fs::write(
        &cfg_path,
        r#"{"schema":1,"enabled":true,"dynamic":false,"base_profile":"balance",
            "app_map":{"com.YoStarEN.AzurLane":"game"}}"#,
    )
    .unwrap();
    d.send(json!({"cmd":"config_changed"}));
    let applied = d.wait_for(
        |v| v["event"] == "applied" && v["profile"] == "balance",
        Duration::from_secs(10),
    );
    assert_eq!(applied["reason"], "base");

    // 7. screen off -> sleep after the grace period (10 s + settle)
    d.send(json!({"cmd":"screen","on":false,"locked":true}));
    let applied = d.wait_for(
        |v| v["event"] == "applied" && v["profile"] == "sleep",
        Duration::from_secs(16),
    );
    assert_eq!(applied["reason"], "screen off");

    // 8. restore (service-off path) then shutdown
    d.send(json!({"cmd":"restore"}));
    let restored = d.wait_for(|v| v["event"] == "restored", Duration::from_secs(10));
    assert_eq!(restored["ok"], true);

    d.send(json!({"cmd":"shutdown"}));
    let bye = d.wait_for(|v| v["event"] == "bye", Duration::from_secs(5));
    assert_eq!(bye["event"], "bye");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn daemon_diag_stats_and_env_events() {
    let dir = tmp("diag");
    let cfg_path = dir.join("config.json");
    std::fs::write(
        &cfg_path,
        r#"{"schema":1,"enabled":true,"dynamic":true,"base_profile":"balance"}"#,
    )
    .unwrap();

    // fake sysfs tree: battery + thermal + GPU (host has no device nodes)
    let root = dir.join("fake-root");
    let w = |rel: &str, body: &str| {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    };
    w("sys/class/power_supply/battery/capacity", "95\n");
    w("sys/class/power_supply/battery/status", "Charging\n");
    w("sys/class/power_supply/battery/temp", "320\n");
    w("sys/class/thermal/thermal_zone24/type", "cpuss-0-usr\n");
    w("sys/class/thermal/thermal_zone24/temp", "38800\n");
    w("sys/class/kgsl/kgsl-3d0/gpu_busy_percentage", "3 %\n");

    let mut d = Daemon::spawn_env(&dir, &cfg_path, &[("MIFINETUNE_SYSFS_ROOT", root.to_str().unwrap())]);

    // startup handshake: hello -> state -> env (the sampler emits immediately)
    let hello = d.wait_for(|v| v["event"] == "hello", Duration::from_secs(5));
    assert_eq!(hello["version"], 1);
    let env_ev = d.wait_for(|v| v["event"] == "env", Duration::from_secs(5));
    assert_eq!(env_ev["env"]["battery_pct"], 95);
    assert_eq!(env_ev["env"]["charging"], true);
    assert_eq!(env_ev["env"]["battery_temp_c"], 32.0);
    assert_eq!(env_ev["env"]["cpu_temp_c"], 38.8);
    assert_eq!(env_ev["env"]["gpu_busy_pct"], 3);

    // one real switch so the history has an entry
    d.send(json!({"cmd":"screen","on":true,"locked":false}));
    d.send(json!({"cmd":"fg","pkg":"com.whatsapp"}));
    d.wait_for(|v| v["event"] == "applied" && v["profile"] == "balance", Duration::from_secs(10));

    d.send(json!({"cmd":"diag"}));
    let diag = d.wait_for(|v| v["event"] == "diag", Duration::from_secs(5));
    assert!(diag["diag"]["pid"].is_u64());
    assert!(diag["diag"]["uptime_s"].is_u64());
    assert_eq!(diag["diag"]["config"]["base_profile"], "balance");
    assert_eq!(diag["diag"]["env"]["battery_pct"], 95);
    assert!(diag["diag"]["watchers"]["fg"].is_boolean());
    assert_eq!(diag["diag"]["holds"]["perf_held"], false);

    d.send(json!({"cmd":"stats"}));
    let stats = d.wait_for(|v| v["event"] == "stats", Duration::from_secs(5));
    let entries = stats["entries"].as_array().expect("entries array");
    assert!(!entries.is_empty(), "one applied switch must be recorded");
    let last = entries.last().unwrap();
    assert_eq!(last["to"], "balance");
    assert_eq!(last["reason"], "base");
    assert_eq!(last["battery"], 95);

    // the history survives on disk for the next daemon
    let on_disk = std::fs::read_to_string(dir.join("stats.json")).unwrap();
    assert!(on_disk.contains("\"balance\""));

    d.send(json!({"cmd":"shutdown"}));
    d.wait_for(|v| v["event"] == "bye", Duration::from_secs(5));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn daemon_guards_react_to_env_changes() {
    let dir = tmp("guards");
    let cfg_path = dir.join("config.json");
    std::fs::write(
        &cfg_path,
        r#"{"schema":1,"enabled":true,"dynamic":true,"base_profile":"balance",
            "app_map":{"com.YoStarEN.AzurLane":"game"}}"#,
    )
    .unwrap();

    // fake sysfs: battery low (10%, discharging), CPU cool
    let root = dir.join("fake-root");
    let w = |rel: &str, body: &str| {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    };
    w("sys/class/power_supply/battery/capacity", "10\n");
    w("sys/class/power_supply/battery/status", "Discharging\n");
    w("sys/class/thermal/thermal_zone24/type", "cpuss-0-usr\n");
    w("sys/class/thermal/thermal_zone24/temp", "35000\n");

    let mut d = Daemon::spawn_env(
        &dir,
        &cfg_path,
        &[
            ("MIFINETUNE_SYSFS_ROOT", root.to_str().unwrap()),
            ("MIFINETUNE_ENV_SAMPLE_MS", "200"),
        ],
    );

    d.wait_for(|v| v["event"] == "hello", Duration::from_secs(5));
    d.wait_for(|v| v["event"] == "env" && v["env"]["battery_pct"] == 10, Duration::from_secs(5));

    // low battery beats the game mapping
    d.send(json!({"cmd":"screen","on":true,"locked":false}));
    d.send(json!({"cmd":"fg","pkg":"com.YoStarEN.AzurLane"}));
    let applied = d.wait_for(
        |v| v["event"] == "applied" && v["profile"] == "powersave",
        Duration::from_secs(10),
    );
    assert_eq!(applied["reason"], "low battery");

    // charged -> the mapping wins again
    w("sys/class/power_supply/battery/capacity", "90\n");
    let applied = d.wait_for(
        |v| v["event"] == "applied" && v["profile"] == "game",
        Duration::from_secs(10),
    );
    assert_eq!(applied["reason"], "app");

    // hot CPU -> game steps down to balance (thermal)
    w("sys/class/thermal/thermal_zone24/temp", "80000\n");
    let applied = d.wait_for(
        |v| v["event"] == "applied" && v["profile"] == "balance",
        Duration::from_secs(10),
    );
    assert_eq!(applied["reason"], "thermal");

    // cooled below the hysteresis band -> game returns
    w("sys/class/thermal/thermal_zone24/temp", "69000\n");
    let applied = d.wait_for(
        |v| v["event"] == "applied" && v["profile"] == "game",
        Duration::from_secs(10),
    );
    assert_eq!(applied["reason"], "app");

    d.send(json!({"cmd":"shutdown"}));
    d.wait_for(|v| v["event"] == "bye", Duration::from_secs(5));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn daemon_exits_on_stdin_eof() {
    let dir = tmp("eof");
    let cfg_path = dir.join("config.json");
    std::fs::write(&cfg_path, "{}").unwrap();

    let bin = env!("CARGO_BIN_EXE_miui-ft");
    let mut child = Command::new(bin)
        .args([
            "serve",
            "--state-dir",
            dir.to_str().unwrap(),
            "--config",
            cfg_path.to_str().unwrap(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    // drop stdin immediately -> EOF -> daemon must exit
    drop(child.stdin.take());
    let status = child.wait_timeout(Duration::from_secs(5)).unwrap();
    assert!(status.is_some(), "daemon must exit when stdin closes");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn doctor_reports_valid_json() {
    let dir = tmp("doctor");
    let bin = env!("CARGO_BIN_EXE_miui-ft");
    let out = Command::new(bin)
        .args(["doctor", "--state-dir", dir.to_str().unwrap()])
        .output()
        .unwrap();
    let v: Value = serde_json::from_slice(&out.stdout).expect("doctor output is JSON");
    assert!(v["ok"].is_boolean());
    let checks = v["checks"].as_array().expect("checks array");
    assert!(checks.iter().any(|c| c["name"] == "catalog"));
    assert!(checks.iter().any(|c| c["name"] == "profiles"));
    let _ = std::fs::remove_dir_all(&dir);
}
