//! `miui-ft` — CLI entry point.
//!
//! Commands: probe | profiles | plan | apply | restore | verify | status | serve | doctor
//! Machine consumers (the Kotlin app) pass `--json`.
//!
//! `serve` is special: it does not return a JSON payload on exit — it runs
//! the stdio daemon (JSON-lines protocol) until stdin closes. See `daemon/`.

use mifinetune_core::engine::apply::{self, Store};
use mifinetune_core::engine::catalog;
use mifinetune_core::engine::doctor;
use mifinetune_core::engine::plan::build_plan;
use mifinetune_core::engine::probe;
use std::path::PathBuf;
use std::process::ExitCode;

struct Args {
    cmd: String,
    id: Option<String>,
    state_dir: PathBuf,
    profiles: Option<PathBuf>,
    /// `serve` only: config.json the app owns (default: <state-dir>/config.json).
    config: Option<PathBuf>,
}

fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut cmd: Option<String> = None;
    let mut id: Option<String> = None;
    let mut state_dir = apply::default_state_dir();
    let mut profiles: Option<PathBuf> = None;
    let mut config: Option<PathBuf> = None;
    let mut i = 0;
    while i < argv.len() {
        let a = &argv[i];
        match a.as_str() {
            "--json" => {} // JSON is the only output format (accepted for compatibility)
            "--state-dir" => {
                i += 1;
                let v = argv.get(i).ok_or("--state-dir needs a value")?;
                state_dir = PathBuf::from(v);
            }
            "--profiles" => {
                i += 1;
                let v = argv.get(i).ok_or("--profiles needs a value")?;
                profiles = Some(PathBuf::from(v));
            }
            "--config" => {
                i += 1;
                let v = argv.get(i).ok_or("--config needs a value")?;
                config = Some(PathBuf::from(v));
            }
            "--help" | "-h" => {
                return Err(String::new()); // usage, not an error
            }
            other if other.starts_with('-') => return Err(format!("unknown flag {other}")),
            other => {
                if cmd.is_none() {
                    cmd = Some(other.to_string());
                } else if id.is_none() {
                    id = Some(other.to_string());
                } else {
                    return Err(format!("unexpected argument {other}"));
                }
            }
        }
        i += 1;
    }
    Ok(Args {
        cmd: cmd.ok_or_else(|| USAGE.to_string())?,
        id,
        state_dir,
        profiles,
        config,
    })
}

const USAGE: &str =
    "usage: miui-ft <command> [--json] [--state-dir DIR] [--profiles FILE] [--config FILE]
commands:
  probe                 read device state (read-only)
  profiles              list bundled/available profiles
  plan <id>             validate a profile against the live device
  apply <id>            snapshot + write + verify (root)
  restore               write snapshot back (root)
  verify <id>           check live values vs profile, detect drift
  status                active profile, snapshot, framework evidence
  serve                 stdio daemon (JSON-lines on stdin/stdout)
  doctor                environment self-check (root/binaries/config/state)
  catalog               dump the full parameter catalog as JSON
  apply runs one automatic re-plan pass when a governor switch reveals
                      previously hidden governor-specific nodes";

fn is_root() -> bool {
    std::fs::read_to_string("/proc/self/status")
        .map(|s| {
            s.lines()
                .find(|l| l.starts_with("Uid:"))
                .and_then(|l| l.split_whitespace().nth(2))
                .map(|euid| euid == "0")
                .unwrap_or(false)
        })
        .unwrap_or(false)
}

fn require_root() -> Result<(), String> {
    if is_root() {
        Ok(())
    } else {
        Err("root required for this command".into())
    }
}

fn run(a: &Args) -> Result<(i32, String), String> {
    let store = Store::new(&a.state_dir);
    match a.cmd.as_str() {
        "probe" => {
            let p = probe::probe();
            let json = serde_json::to_string_pretty(&p).map_err(|e| e.to_string())?;
            Ok((0, json))
        }
        "profiles" => {
            let files = store.load_profiles(a.profiles.as_deref())?;
            let json = serde_json::to_string_pretty(
                &files
                    .profiles
                    .iter()
                    .map(|p| {
                        serde_json::json!({
                            "id": p.id, "label": p.label, "desc": p.desc, "params": p.params.len()
                        })
                    })
                    .collect::<Vec<_>>(),
            )
            .map_err(|e| e.to_string())?;
            Ok((0, json))
        }
        "plan" | "apply" | "verify" => {
            let id =
                a.id.as_deref()
                    .ok_or_else(|| format!("missing profile id\n{USAGE}"))?;
            if a.cmd == "apply" {
                require_root()?;
            }
            let files = store.load_profiles(a.profiles.as_deref())?;
            let profile = files.profiles.iter().find(|p| p.id == id).ok_or_else(|| {
                format!(
                    "unknown profile '{id}' (have: {})",
                    files
                        .profiles
                        .iter()
                        .map(|p| p.id.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })?;
            match a.cmd.as_str() {
                "plan" => {
                    let p = probe::probe();
                    let plan = build_plan(profile, &p);
                    let json = serde_json::to_string_pretty(&plan).map_err(|e| e.to_string())?;
                    let code = if plan.ok { 0 } else { 1 };
                    Ok((code, json))
                }
                "verify" => {
                    let p = probe::probe();
                    let plan = build_plan(profile, &p);
                    let report = apply::verify_plan(&store, &plan);
                    let json = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
                    let code = if report.ok { 0 } else { 2 };
                    Ok((code, json))
                }
                _ => {
                    // apply probes + plans internally (pass-2 included)
                    let report = apply::apply_with_pass2(&store, profile, true)?;
                    let json = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
                    let code = if report.ok { 0 } else { 2 };
                    Ok((code, json))
                }
            }
        }
        "restore" => {
            require_root()?;
            let p = probe::probe();
            let report = apply::restore(&store, &p)?;
            let json = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
            let code = if report.ok { 0 } else { 2 };
            Ok((code, json))
        }
        "catalog" => {
            // Full parameter catalog (used by tools/owner-map-audit.sh and
            // for UI debugging): every writable key with tier + constraints.
            let entries: Vec<serde_json::Value> = catalog::catalog()
                .iter()
                .map(|e| {
                    serde_json::json!({
                        "key": e.key, "path": e.path, "tier": e.tier,
                        "kind": format!("{:?}", e.kind), "scope": e.scope,
                        "range": e.range, "min_cpus_of": e.min_cpus_of,
                    })
                })
                .collect();
            let payload = serde_json::to_string_pretty(&entries).map_err(|e| e.to_string())?;
            Ok((0, payload))
        }
        "doctor" => {
            // environment self-check (device debugging without the app)
            let config = a
                .config
                .clone()
                .unwrap_or_else(|| a.state_dir.join("config.json"));
            let (ok, payload) = doctor::run(&a.state_dir, Some(&config));
            let json = serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())?;
            Ok((if ok { 0 } else { 1 }, json))
        }
        "status" => {
            let state = store.load_state();
            let snap = store.load_snapshot();
            let files = store.load_profiles(a.profiles.as_deref())?;
            let p = probe::probe();
            let status = serde_json::json!({
                "state_dir": store.dir(),
                "active": state.active,
                "updated": state.updated,
                "last_mode": state.last_mode,
                "snapshot": snap.as_ref().map(|s| serde_json::json!({
                    "created": s.created, "keys": s.values.len(), "device": s.device,
                })),
                "profiles": files.profiles.iter().map(|x| &x.id).collect::<Vec<_>>(),
                "device": p.device,
                "framework": p.framework,
                "root": is_root(),
                "catalog": {
                    "total": catalog::catalog().len(),
                    "free": catalog::catalog().iter().filter(|e| e.tier == catalog::Tier::Free).count(),
                    "baseline": catalog::catalog().iter().filter(|e| e.tier == catalog::Tier::Baseline).count(),
                    "present": p.entries.values().filter(|e| e.exists).count(),
                },
            });
            let json = serde_json::to_string_pretty(&status).map_err(|e| e.to_string())?;
            Ok((0, json))
        }
        other => Err(format!("unknown command '{other}'\n{USAGE}")),
    }
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = match parse_args(&argv) {
        Ok(a) => a,
        Err(e) => {
            if e.is_empty() || e == USAGE {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            eprintln!("{e}");
            return ExitCode::from(1);
        }
    };

    match args.cmd.as_str() {
        "serve" => {
            // stdio daemon: streams JSON-lines until stdin closes; never
            // prints a one-shot payload (see daemon/mod.rs).
            let config = args
                .config
                .clone()
                .unwrap_or_else(|| args.state_dir.join("config.json"));
            match mifinetune_core::daemon::run(&args.state_dir, &config) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("error: {e}");
                    ExitCode::from(1)
                }
            }
        }
        _ => match run(&args) {
            Ok((code, payload)) => {
                // JSON is the canonical format for every command (the app parses
                // it; humans get the same pretty-printed payload).
                println!("{payload}");
                ExitCode::from(code.clamp(0, 255) as u8)
            }
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::from(1)
            }
        },
    }
}
