//! Apply / restore / verify — the only code allowed to write, always guarded.
//!
//! Files:
//! - `store.rs`   state dir: snapshot/state JSON (atomic) + profile loading
//! - `write.rs`   apply a validated plan (ordered, read-back verified)
//! - `restore.rs` write the stock snapshot back (engine OFF)
//! - `verify.rs`  drift detection (read-only)
//!
//! Non-goals: deciding values (`plan.rs`), UI.

mod restore;
mod store;
mod verify;
mod write;

pub use restore::restore;
pub use store::Store;
pub use verify::verify_plan;
pub use write::{apply_plan, apply_with_pass2};

/// Guarded single write (catalog guard + node write) for daemon-side
/// maintenance windows — same gate as every engine write.
pub fn guarded_write(path: &str, value: &str) -> Result<(), String> {
    write::write_one(path, value)
}

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn default_state_dir() -> PathBuf {
    std::env::var("MIFINETUNE_STATE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/data/adb/mifinetune"))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapValue {
    pub path: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub created: u64,
    pub device: String,
    pub values: BTreeMap<String, SnapValue>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct State {
    pub active: Option<String>,
    pub updated: u64,
    #[serde(default)]
    pub last_mode: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct WriteResult {
    pub key: String,
    pub path: String,
    pub resolved: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
    pub written: bool,
    pub verified: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LockedKey {
    pub key: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ApplyReport {
    pub mode: String, // "apply" | "restore"
    pub profile: Option<String>,
    pub wrote: usize,
    pub unchanged: usize,
    pub verified: usize,
    pub failed: usize,
    pub locked: Vec<LockedKey>,
    pub results: Vec<WriteResult>,
    pub ok: bool,
    pub snapshot_created: bool,
    pub active: Option<String>,
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
}

/// Atomic JSON write: tmp file in the same directory + fsync + rename, so a
/// crash mid-write can never truncate snapshot.json/state.json (a lost
/// snapshot means a lost restore path).
fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("mkdir {parent:?}: {e}"))?;
    }
    let s = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    {
        let mut f = fs::File::create(&tmp).map_err(|e| format!("create {tmp:?}: {e}"))?;
        f.write_all(s.as_bytes())
            .map_err(|e| format!("write {tmp:?}: {e}"))?;
        f.sync_all().map_err(|e| format!("fsync {tmp:?}: {e}"))?;
    }
    fs::rename(&tmp, path).map_err(|e| format!("rename {tmp:?} -> {path:?}: {e}"))
}
