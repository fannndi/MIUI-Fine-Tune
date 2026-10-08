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

struct Report {
    ok: bool,
    checks: Vec<Value>,
}

impl Report {
    fn new() -> Self {
        Report { ok: true, checks: Vec::new() }
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
    r.check("root", euid_is_root(), true, json!({ "hint": "run via su" }));

    // --- helper binaries (absolute paths; the su context has no PATH) -------
    for bin in ["/system/bin/logcat", "/system/bin/settings", "/system/bin/getprop"] {
        r.check(
            "binary",
            Path::new(bin).exists(),
            false,
            json!({ "path": bin, "exists": Path::new(bin).exists() }),
        );
    }

    // --- state dir ------------------------------------------------------------
    let dir_ok = fs::create_dir_all(state_dir).is_ok();
    let dir_writable = dir_ok && fs::metadata(state_dir).map(|m| !m.permissions().readonly()).unwrap_or(false);
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
                Err(e) => r.check("config", false, false, json!({ "path": cp.display().to_string(), "error": e.to_string() })),
            },
            Err(e) => r.check("config", false, false, json!({ "path": cp.display().to_string(), "error": e.to_string() })),
        }
    }

    // --- holds.json (bridge crash recovery) ----------------------------------------
    let holds = state_dir.join("holds.json");
    if holds.exists() {
        let parsed = fs::read_to_string(&holds)
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok());
        r.check("holds", parsed.is_some(), false, json!({ "path": holds.display().to_string() }));
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
