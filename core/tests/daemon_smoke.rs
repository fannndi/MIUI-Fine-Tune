//! End-to-end daemon smoke tests on the host: spawn the real `miui-ft serve`
//! binary, exchange JSON-lines, assert the decision pipeline.
//!
//! Runs against fake sysfs (host): every engine op reports "node missing"
//! (locked), which is a clean apply — perfect for exercising the protocol,
//! arbiter, worker, config, bridge and env-guard paths without a device.

mod common;

use common::{tmp, Daemon, WaitTimeout};
use serde_json::{json, Value};
use std::process::{Command, Stdio};
use std::time::Duration;

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
    d.wait_for(
        |v| v["event"] == "applied" && v["profile"] == "game",
        Duration::from_secs(10),
    );

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
        let p = root.join(rel.trim_start_matches('/'));
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    };
    w("sys/class/power_supply/battery/capacity", "95\n");
    w("sys/class/power_supply/battery/status", "Charging\n");
    w("sys/class/power_supply/battery/temp", "320\n");
    w("sys/class/thermal/thermal_zone24/type", "cpuss-0-usr\n");
    w("sys/class/thermal/thermal_zone24/temp", "38800\n");
    w("sys/class/kgsl/kgsl-3d0/gpu_busy_percentage", "3 %\n");

    let mut d = Daemon::spawn_env(
        &dir,
        &cfg_path,
        &[("MIFINETUNE_SYSFS_ROOT", root.to_str().unwrap())],
    );

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
    d.wait_for(
        |v| v["event"] == "applied" && v["profile"] == "balance",
        Duration::from_secs(10),
    );

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
    d.wait_for(
        |v| v["event"] == "env" && v["env"]["battery_pct"] == 10,
        Duration::from_secs(5),
    );

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
fn profile_pack_change_triggers_reevaluation() {
    let dir = tmp("profiles");
    let cfg_path = dir.join("config.json");
    std::fs::write(
        &cfg_path,
        r#"{"schema":1,"enabled":true,"dynamic":true,"base_profile":"balance"}"#,
    )
    .unwrap();

    let mut d = Daemon::spawn(&dir, &cfg_path);
    d.wait_for(|v| v["event"] == "hello", Duration::from_secs(5));
    d.send(json!({"cmd":"screen","on":true,"locked":false}));
    d.send(json!({"cmd":"fg","pkg":"com.whatsapp"}));
    d.wait_for(
        |v| v["event"] == "applied" && v["profile"] == "balance",
        Duration::from_secs(10),
    );

    // the app deploys a new profile pack while the daemon runs
    std::fs::write(
        dir.join("profiles.json"),
        r#"{"schema":1,"name":"test","device":"surya","rom":"test","profiles":[
            {"id":"balance","label":"B","desc":"","params":{"kernel.sched_nr_migrate":"24"}},
            {"id":"sleep","label":"S","desc":"","params":{"kernel.sched_nr_migrate":"8"}}]}"#,
    )
    .unwrap();

    // the mtime poll must re-evaluate with the new pack (no switch needed)
    let dec = d.wait_for(
        |v| v["event"] == "decision" && v["trigger"] == "profiles",
        Duration::from_secs(10),
    );
    assert_eq!(dec["action"], "apply");
    assert_eq!(dec["profile"], "balance");

    d.send(json!({"cmd":"shutdown"}));
    d.wait_for(|v| v["event"] == "bye", Duration::from_secs(5));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn maintenance_triggers_while_charging_and_idle() {
    let dir = tmp("maint");
    let cfg_path = dir.join("config.json");
    std::fs::write(
        &cfg_path,
        r#"{"schema":1,"enabled":true,"dynamic":true,"base_profile":"balance","maintenance":true}"#,
    )
    .unwrap();

    // fake sysfs: charging (f2fs nodes intentionally absent -> reported skip)
    let root = dir.join("fake-root");
    let w = |rel: &str, body: &str| {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    };
    w("sys/class/power_supply/battery/capacity", "80\n");
    w("sys/class/power_supply/battery/status", "Charging\n");

    let mut d = Daemon::spawn_env(
        &dir,
        &cfg_path,
        &[
            ("MIFINETUNE_SYSFS_ROOT", root.to_str().unwrap()),
            ("MIFINETUNE_ENV_SAMPLE_MS", "200"),
            ("MIFINETUNE_MAINT_MIN_OFF_SECS", "0"),
            ("MIFINETUNE_MAINT_POLL_MS", "100"),
            ("MIFINETUNE_MAINT_MAX_SECS", "2"),
        ],
    );

    d.wait_for(|v| v["event"] == "hello", Duration::from_secs(5));
    // screen off, charging -> the weekly window runs (first run: last = 0)
    d.send(json!({"cmd":"screen","on":false,"locked":true}));
    let ev = d.wait_for(|v| v["event"] == "maintenance", Duration::from_secs(10));
    assert_eq!(ev["ok"], false, "host has no f2fs nodes");
    assert!(ev["detail"].as_str().unwrap_or("").contains("unavailable"));

    // bookkeeping persisted so the next run is a week away
    let f = std::fs::read_to_string(dir.join("maintenance.json")).unwrap();
    assert!(f.contains("unavailable"), "file: {f}");

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

// ===== v0.11: reconcile + auto-revive watchdog =====

/// Write the fake tree nodes used by the reconcile E2E. Values start as a
/// realistic stale mix (a leftover from an old game/boost apply) so the first
/// forced apply must heal every union key.
fn reconciled_tree(dir: &std::path::Path) -> std::path::PathBuf {
    let root = dir.join("fake-root");
    let w = |rel: &str, body: &str| {
        let p = root.join(rel.trim_start_matches('/'));
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    };
    let stock: &[(&str, &str)] = &[
        // cpusets (capacity-wise identical to stock MIUI)
        ("/dev/cpuset/background/cpus", "0-5"),
        ("/dev/cpuset/foreground/cpus", "0-7"),
        ("/dev/cpuset/system-background/cpus", "0-5"),
        ("/dev/cpuset/top-app/cpus", "0-7"),
        // l3-lat floors
        (
            "/sys/class/devfreq/soc:qcom,cpu0-cpu-l3-lat/min_freq",
            "300000000",
        ),
        (
            "/sys/class/devfreq/soc:qcom,cpu6-cpu-l3-lat/min_freq",
            "300000000",
        ),
        // GPU levels
        ("/sys/class/kgsl/kgsl-3d0/min_pwrlevel", "6"),
        ("/sys/class/kgsl/kgsl-3d0/max_pwrlevel", "0"),
        ("sys/class/kgsl/kgsl-3d0/min_pwrlevel", "6"),
        ("sys/class/kgsl/kgsl-3d0/max_pwrlevel", "0"),
        // io
        ("sys/block/sda/queue/iostats", "1"),
        ("sys/block/sda/queue/nomerges", "0"),
        ("sys/block/sda/queue/nr_requests", "128"),
        ("sys/block/sda/queue/read_ahead_kb", "512"),
        ("sys/block/sda/queue/scheduler", "noop deadline [cfq]"),
        // kernel sysctls (stale mix: leftover up/downmigrate + pl)
        ("proc/sys/kernel/sched_conservative_pl", "1"),
        ("proc/sys/kernel/sched_downmigrate", "50"),
        ("proc/sys/kernel/sched_latency_ns", "6000000"),
        ("proc/sys/kernel/sched_migration_cost_ns", "500000"),
        ("proc/sys/kernel/sched_min_granularity_ns", "1000000"),
        ("proc/sys/kernel/sched_min_task_util_for_boost", "51"),
        ("proc/sys/kernel/sched_min_task_util_for_colocation", "35"),
        ("proc/sys/kernel/sched_nr_migrate", "8"),
        ("proc/sys/kernel/sched_upmigrate", "60"),
        ("proc/sys/kernel/sched_wakeup_granularity_ns", "500000"),
        // policy0 (cpubw shares cluster cpus, stale mix too)
        ("sys/devices/system/cpu/cpu0/core_ctl/max_cpus", "6"),
        ("sys/devices/system/cpu/cpu0/core_ctl/min_cpus", "6"),
        ("sys/devices/system/cpu/cpu0/core_ctl/task_thres", "12"),
        (
            "sys/devices/system/cpu/cpufreq/policy0/scaling_governor",
            "schedutil",
        ),
        (
            "sys/devices/system/cpu/cpufreq/policy0/scaling_max_freq",
            "1248000",
        ),
        (
            "sys/devices/system/cpu/cpufreq/policy0/scaling_min_freq",
            "768000",
        ),
        (
            "sys/devices/system/cpu/cpufreq/policy0/schedutil/hispeed_freq",
            "1497600",
        ),
        (
            "sys/devices/system/cpu/cpufreq/policy0/schedutil/hispeed_load",
            "75",
        ),
        (
            "sys/devices/system/cpu/cpufreq/policy0/schedutil/up_rate_limit_us",
            "0",
        ),
        (
            "sys/devices/system/cpu/cpufreq/policy0/schedutil/down_rate_limit_us",
            "0",
        ),
        ("sys/devices/system/cpu/cpufreq/policy0/schedutil/pl", "1"),
        // policy6 (stale legacy)
        ("sys/devices/system/cpu/cpu6/core_ctl/min_cpus", "1"),
        (
            "sys/devices/system/cpu/cpufreq/policy6/scaling_governor",
            "schedutil",
        ),
        (
            "sys/devices/system/cpu/cpufreq/policy6/scaling_max_freq",
            "1555200",
        ),
        (
            "sys/devices/system/cpu/cpufreq/policy6/scaling_min_freq",
            "1094400",
        ),
        (
            "sys/devices/system/cpu/cpufreq/policy6/schedutil/hispeed_freq",
            "1555200",
        ),
        (
            "sys/devices/system/cpu/cpufreq/policy6/schedutil/hispeed_load",
            "85",
        ),
        (
            "sys/devices/system/cpu/cpufreq/policy6/schedutil/up_rate_limit_us",
            "0",
        ),
        (
            "sys/devices/system/cpu/cpufreq/policy6/schedutil/down_rate_limit_us",
            "0",
        ),
        ("sys/devices/system/cpu/cpufreq/policy6/schedutil/pl", "1"),
        // stune
        ("/dev/stune/top-app/boost", "0"),
        // probe options: OPP / governor / cluster lists + related cpus
        (
            "sys/devices/system/cpu/cpufreq/policy0/scaling_available_governors",
            "powersave schedutil",
        ),
        (
            "sys/devices/system/cpu/cpufreq/policy0/scaling_available_frequencies",
            "300000 576000 768000 1017600 1248000 1324800 1497600 1612800 1804800",
        ),
        (
            "sys/devices/system/cpu/cpufreq/policy0/related_cpus",
            "0 1 2 3 4 5",
        ),
        (
            "sys/devices/system/cpu/cpufreq/policy6/scaling_available_governors",
            "powersave schedutil",
        ),
        (
            "sys/devices/system/cpu/cpufreq/policy6/scaling_available_frequencies",
            "300000 652800 806400 979200 1094400 1209600 1324800 1555200 1708800 2304000",
        ),
        ("sys/devices/system/cpu/cpufreq/policy6/related_cpus", "6 7"),
        (
            "sys/class/kgsl/kgsl-3d0/devfreq/available_governors",
            "simple_ondemand msm-adreno-tz powersave",
        ),
        (
            "sys/class/kgsl/kgsl-3d0/gpu_available_frequencies",
            "180000000 266000000 305000000 380000000 430000000 575000000 825000000",
        ),
        (
            "proc/sys/net/ipv4/tcp_available_congestion_control",
            "cubic reno",
        ),
        // vm
        ("proc/sys/vm/dirty_background_ratio", "10"),
        ("proc/sys/vm/dirty_expire_centisecs", "600"),
        ("proc/sys/vm/dirty_ratio", "30"),
        ("proc/sys/vm/dirty_writeback_centisecs", "1500"),
        ("proc/sys/vm/stat_interval", "3"),
        ("proc/sys/vm/vfs_cache_pressure", "50"),
    ];
    for (rel, body) in stock {
        if *rel == "ext4" {
            continue;
        }
        w(rel, body);
    }
    // stune boost file direct
    w("/dev/stune/top-app/boost", "0");
    root
}

fn read_tree_node(root: &std::path::Path, rel: &str) -> String {
    std::fs::read_to_string(root.join(rel))
        .unwrap_or_else(|_| "<missing>".into())
        .trim()
        .to_string()
}

#[test]
fn watchdog_and_force_apply_reconcile_every_profile_key() {
    let dir = tmp("tok");
    let cfg_path = dir.join("config.json");
    std::fs::write(
        &cfg_path,
        r#"{"schema":1,"enabled":true,"dynamic":true,"base_profile":"balance"}"#,
    )
    .unwrap();
    let root = reconciled_tree(&dir);

    let sysfs_root = root.to_str().unwrap().to_owned();
    let envs: Vec<(&str, &str)> = vec![
        ("MIFINETUNE_SYSFS_ROOT", &sysfs_root),
        ("MIFINETUNE_WATCHDOG_SECS", "2"),
        ("MIFINETUNE_ENV_SAMPLE_MS", "1000"),
    ];
    let mut d = Daemon::spawn_env(&dir, &cfg_path, &envs);
    d.send(serde_json::json!({"cmd": "hello"}));
    d.wait_for(|v| v["event"] == "hello", Duration::from_secs(5));

    // screen on + whatsapp -> balance (first apply of the run = force)
    d.send(serde_json::json!({"cmd": "screen", "on": true, "locked": false}));
    d.send(serde_json::json!({"cmd": "fg", "pkg": "app.balance"}));
    d.wait_for(
        |v| v["event"] == "applied" && v["profile"] == "balance",
        Duration::from_secs(10),
    );

    // forced apply side effects: every PROFILE key reaches the profile value
    assert_eq!(
        read_tree_node(
            &root,
            "sys/devices/system/cpu/cpufreq/policy0/scaling_min_freq"
        ),
        "576000"
    );
    assert_eq!(
        read_tree_node(
            &root,
            "sys/devices/system/cpu/cpufreq/policy6/scaling_min_freq"
        ),
        "652800"
    );
    assert_eq!(
        read_tree_node(&root, "sys/devices/system/cpu/cpu0/core_ctl/task_thres"),
        "8"
    );
    // union keys that balance doesn't set get stock back (vm.pl)
    assert_eq!(
        read_tree_node(&root, "proc/sys/kernel/sched_conservative_pl"),
        "0"
    );
    // cpu6 core_ctl stays untouched (not in any profile params)
    assert_eq!(
        read_tree_node(&root, "sys/devices/system/cpu/cpu6/core_ctl/min_cpus"),
        "1"
    );

    // external drift now: MIUI/interference pokes a stale value
    std::fs::write(
        root.join("sys/devices/system/cpu/cpufreq/policy6/scaling_min_freq"),
        "1094400\n",
    )
    .unwrap();

    // watchdog fires within ~3s (2s interval >= one 3s supervise pass)
    d.wait_for(
        |v| {
            v["event"] == "applied"
                && v["profile"] == "balance"
                && v["wrote"].as_u64().unwrap_or(0) >= 1
        },
        Duration::from_secs(10),
    );
    assert_eq!(
        read_tree_node(
            &root,
            "sys/devices/system/cpu/cpufreq/policy6/scaling_min_freq"
        ),
        "652800",
        "watchdog reconcile must heal the drifted freq floor"
    );
    d.send(serde_json::json!({"cmd": "shutdown"}));
    d.wait_for(|v| v["event"] == "bye", Duration::from_secs(5));
    let _ = std::fs::remove_dir_all(&dir);
}
