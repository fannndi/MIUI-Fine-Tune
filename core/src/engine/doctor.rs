//! `doctor` — one-shot environment self-check for device debugging.
//!
//! Reports what the daemon needs to function: root, helper binaries, state
//! dir, catalog presence, profile pack, config/holds parse. JSON output so
//! tools and the app can consume it; the exit code is non-zero when any
//! critical check fails.

use crate::engine::apply::Store;
use crate::engine::probe;
use crate::engine::profile::parse_profiles;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use std::process::Command;

struct Report {
    ok: bool,
    checks: Vec<Value>,
}

impl Report {
    fn new() -> Self {
        Report {
            ok: true,
            checks: Vec::new(),
        }
    }

    fn check(&mut self, name: &str, pass: bool, critical: bool, detail: Value) {
        if critical && !pass {
            self.ok = false;
        }
        self.checks.push(json!({
            "name": name,
            "pass": pass,
            "critical": critical,
            "detail": detail,
        }));
    }
}

fn euid_is_root() -> bool {
    fs::read_to_string("/proc/self/status")
        .map(|s| {
            s.lines()
                .find(|l| l.starts_with("Uid:"))
                .and_then(|l| l.split_whitespace().nth(2))
                .map(|euid| euid == "0")
                .unwrap_or(false)
        })
        .unwrap_or(false)
}

pub fn run(state_dir: &Path, config_path: Option<&Path>) -> (bool, Value) {
    let mut r = Report::new();

    // --- root ---------------------------------------------------------------
    r.check(
        "root",
        euid_is_root(),
        true,
        json!({ "hint": "run via su" }),
    );

    // --- helper binaries (absolute paths; the su context has no PATH) -------
    for bin in [
        "/system/bin/logcat",
        "/system/bin/settings",
        "/system/bin/getprop",
    ] {
        r.check(
            "binary",
            Path::new(bin).exists(),
            false,
            json!({ "path": bin, "exists": Path::new(bin).exists() }),
        );
    }

    // --- state dir ------------------------------------------------------------
    let dir_ok = fs::create_dir_all(state_dir).is_ok();
    let dir_writable = dir_ok
        && fs::metadata(state_dir)
            .map(|m| !m.permissions().readonly())
            .unwrap_or(false);
    r.check(
        "state_dir",
        dir_writable,
        true,
        json!({ "path": state_dir.display().to_string(), "writable": dir_writable }),
    );

    // --- profile pack ----------------------------------------------------------
    match parse_profiles(crate::DEFAULT_PROFILES_JSON) {
        Ok(f) => r.check(
            "profiles",
            true,
            true,
            json!({ "count": f.profiles.len(), "ids": f.profiles.iter().map(|p| &p.id).collect::<Vec<_>>() }),
        ),
        Err(e) => r.check("profiles", false, true, json!({ "error": e })),
    }

    // --- catalog presence on this device ---------------------------------------
    let p = probe::probe();
    let present = p.entries.values().filter(|e| e.exists).count();
    let total = p.entries.len();
    r.check(
        "catalog",
        present > total / 2, // a healthy surya shows most nodes present
        false,
        json!({ "present": present, "total": total, "device": p.device.device }),
    );

    // --- environment telemetry (diag + future env-aware guards) ----------------
    let root = crate::engine::env::default_root();
    let sampler = crate::engine::env::Sampler::new(&root);
    let env = sampler.sample();
    r.check(
        "env_battery",
        env.battery_pct.is_some() && env.charging.is_some(),
        false,
        json!({ "pct": env.battery_pct, "charging": env.charging, "temp_c": env.battery_temp_c }),
    );
    r.check(
        "env_thermal",
        env.cpu_temp_c.is_some(),
        false,
        json!({ "cpu_c": env.cpu_temp_c, "gpu_c": env.gpu_temp_c }),
    );
    r.check(
        "env_gpu",
        env.gpu_busy_pct.is_some(),
        false,
        json!({ "busy_pct": env.gpu_busy_pct }),
    );

    // --- future knob surface (informational; Phase 12 candidates) --------------
    let optional = [
        ("f2fs_gc_urgent", "sys/fs/f2fs/sda16/gc_urgent"),
        ("devfreq_cpubw", "sys/class/devfreq/soc:qcom,cpubw"),
        ("kgsl_idle_timer", "sys/class/kgsl/kgsl-3d0/idle_timer"),
        ("io_read_ahead_kb", "sys/block/sda/queue/read_ahead_kb"),
        ("refresh_rate_key", "sys/class/graphics"),
    ];
    let present: Vec<Value> = optional
        .iter()
        .map(|(n, rel)| json!({ "name": n, "exists": root.join(rel).exists() }))
        .collect();
    r.check("future_knobs", true, false, json!({ "paths": present }));

    // --- refresh-rate settings key (bridge candidate) ---------------------------
    let refresh = Command::new("/system/bin/settings")
        .args(["get", "system", "user_refresh_rate"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty() && s != "null");
    r.check(
        "refresh_rate_key",
        refresh.is_some(),
        false,
        json!({ "value": refresh }),
    );

    // --- logcat epoch format (freshness arithmetic) -----------------------------
    let epoch_ok = Command::new("/system/bin/logcat")
        .args(["-b", "events", "-v", "epoch", "-d", "-t", "5"])
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout).lines().any(|l| {
                l.split_whitespace()
                    .next()
                    .and_then(|t| t.split('.').next())
                    .and_then(|s| s.parse::<u64>().ok())
                    .map(|v| v > 1_000_000_000)
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false);
    r.check(
        "logcat_epoch",
        epoch_ok,
        false,
        json!({ "hint": "-v epoch must print epoch.seconds as the first token" }),
    );

    // --- config.json -------------------------------------------------------------
    if let Some(cp) = config_path {
        match fs::read_to_string(cp) {
            Ok(raw) => match serde_json::from_str::<crate::daemon::DaemonConfig>(&raw) {
                Ok(cfg) => r.check(
                    "config",
                    true,
                    false,
                    json!({
                        "path": cp.display().to_string(),
                        "enabled": cfg.enabled,
                        "dynamic": cfg.dynamic,
                        "base": cfg.base_profile,
                        "mapped": cfg.app_map.len(),
                    }),
                ),
                Err(e) => r.check(
                    "config",
                    false,
                    false,
                    json!({ "path": cp.display().to_string(), "error": e.to_string() }),
                ),
            },
            Err(e) => r.check(
                "config",
                false,
                false,
                json!({ "path": cp.display().to_string(), "error": e.to_string() }),
            ),
        }
    }

    // --- holds.json (bridge crash recovery) ----------------------------------------
    let holds = state_dir.join("holds.json");
    if holds.exists() {
        let parsed = fs::read_to_string(&holds)
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok());
        r.check(
            "holds",
            parsed.is_some(),
            false,
            json!({ "path": holds.display().to_string() }),
        );
    } else {
        r.check("holds", true, false, json!({ "present": false }));
    }

    // --- engine state ---------------------------------------------------------------
    let store = Store::new(state_dir);
    let st = store.load_state();
    let snap = store.load_snapshot();
    r.check(
        "engine_state",
        true,
        false,
        json!({
            "active": st.active,
            "snapshot": snap.map(|s| json!({ "created": s.created, "keys": s.values.len() })),
        }),
    );

    (r.ok, json!({ "ok": r.ok, "checks": r.checks }))
}
