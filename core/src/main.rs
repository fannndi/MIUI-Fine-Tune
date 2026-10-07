//! `miui-ft` — CLI entry point.
//!
//! Commands: probe | profiles | plan | apply | restore | verify | status
//! Machine consumers (the Kotlin app) pass `--json`.

use mifinetune_core::apply::{self, Store};
use mifinetune_core::catalog;
use mifinetune_core::probe;
use mifinetune_core::profile::build_plan;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Out {
    /// JSON is the only output format; `--json` is accepted as a no-op flag.
    Json,
}

struct Args {
    cmd: String,
    id: Option<String>,
    out: Out,
    state_dir: PathBuf,
    profiles: Option<PathBuf>,
}

fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut cmd: Option<String> = None;
    let mut id: Option<String> = None;
    let mut out = Out::Json;
    let mut state_dir = apply::default_state_dir();
    let mut profiles: Option<PathBuf> = None;
    let mut i = 0;
    while i < argv.len() {
        let a = &argv[i];
        match a.as_str() {
            "--json" => out = Out::Json,
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
        out,
        state_dir,
        profiles,
    })
}

const USAGE: &str = "usage: miui-ft <command> [--json] [--state-dir DIR] [--profiles FILE]
commands:
  probe                 read device state (read-only)
  profiles              list bundled/available profiles
  plan <id>             validate a profile against the live device
  apply <id>            snapshot + write + verify (root)
  restore               write snapshot back (root)
  verify <id>           check live values vs profile, detect drift
  status                active profile, snapshot, framework evidence
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

fn emit_json<T: serde::Serialize>(v: &T) {
    println!("{}", serde_json::to_string_pretty(v).expect("serialize"));
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
            let json = serde_json::to_string_pretty(&files.profiles.iter().map(|p| {
                serde_json::json!({
                    "id": p.id, "label": p.label, "desc": p.desc, "params": p.params.len()
                })
            }).collect::<Vec<_>>())
            .map_err(|e| e.to_string())?;
            Ok((0, json))
        }
        "plan" | "apply" | "verify" => {
            let id = a.id.as_deref().ok_or_else(|| format!("missing profile id\n{USAGE}"))?;
            if a.cmd == "apply" {
                require_root()?;
            }
            let files = store.load_profiles(a.profiles.as_deref())?;
            let profile = files
                .profiles
                .iter()
                .find(|p| p.id == id)
                .ok_or_else(|| format!("unknown profile '{id}' (have: {})",
                    files.profiles.iter().map(|p| p.id.as_str()).collect::<Vec<_>>().join(", ")))?;
            let p = probe::probe();
            let plan = build_plan(profile, &p);
            match a.cmd.as_str() {
                "plan" => {
                    let json = serde_json::to_string_pretty(&plan).map_err(|e| e.to_string())?;
                    let code = if plan.ok { 0 } else { 1 };
                    Ok((code, json))
                }
                "verify" => {
                    let report = apply::verify_plan(&store, &plan);
                    let json = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
                    let code = if report.ok { 0 } else { 2 };
                    Ok((code, json))
                }
                _ => {
                    let mut report = apply::apply_plan(&store, &plan, &p)?;
                    // Governor switches materialize governor-specific tunables
                    // (policyN/schedutil/ only exists while schedutil is the
                    // active governor) — one automatic re-plan pass picks up
                    // keys that were "missing" under the previous governor.
                    if report.locked.iter().any(|l| l.reason.contains("node missing")) {
                        let p2 = probe::probe();
                        let plan2 = build_plan(profile, &p2);
                        if plan2.ok {
                            let r2 = apply::apply_plan(&store, &plan2, &p2)?;
                            report.wrote += r2.wrote;
                            report.verified += r2.verified;
                            report.failed += r2.failed;
                            report.results.extend(r2.results);
                            report.locked = r2.locked; // fresh truth per key
                            report.ok = report.ok && r2.ok;
                            if r2.active.is_some() {
                                report.active = r2.active;
                            }
                        }
                    }
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
            if e.is_empty() || e == USAGE.to_string() {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            eprintln!("{e}");
            return ExitCode::from(1);
        }
    };

    match run(&args) {
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
    }
}
