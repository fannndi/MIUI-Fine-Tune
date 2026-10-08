//! Storage maintenance: bounded f2fs GC while charging + idle.
//!
//! Mirrors the ROM's own `/vendor/bin/checkpoint_gc` (AOSP): shorten
//! `gc_urgent_sleep_time`, set `gc_urgent=1`, poll `dirty_segments` until
//! the clean threshold, then restore everything — bounded by [`MAX_RUN_SECS`]
//! for power safety.
//!
//! Trigger: config `maintenance` ON, charging, screen off for at least
//! [`MIN_SCREEN_OFF_SECS`] and the last run older than [`INTERVAL_SECS`].
//! Runs on its own thread; the main loop only receives the result message.
//! All node writes pass the engine's catalog guard (Baseline tier).

use crate::engine::apply::guarded_write;
use crate::engine::env::default_root;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Weekly cadence (AOSP's checkpoint_gc only runs at boot).
pub const INTERVAL_SECS: u64 = 7 * 24 * 3600;
/// Only run when GC has something to do (AOSP threshold).
pub const DIRTY_THRESHOLD: u64 = 100;
/// Hard cap per run (AOSP allows one hour at boot; charging window here).
pub const MAX_RUN_SECS: u64 = 600;
/// Poll cadence inside a run.
pub const POLL_MS: u64 = 5_000;
/// Urgent GC sleep time during a run (AOSP uses 50).
pub const URGENT_SLEEP_MS: u32 = 50;
/// Minimum time the screen must stay off before a run starts (env override
/// `MIFINETUNE_MAINT_MIN_OFF_SECS` for host E2E).
pub const MIN_SCREEN_OFF_SECS: u64 = 60;

// Env overrides: host E2E drives the same code path quickly.
const POLL_MS_ENV: &str = "MIFINETUNE_MAINT_POLL_MS";
const MAX_SECS_ENV: &str = "MIFINETUNE_MAINT_MAX_SECS";
const MIN_OFF_SECS_ENV: &str = "MIFINETUNE_MAINT_MIN_OFF_SECS";

pub fn min_off_secs() -> u64 {
    std::env::var(MIN_OFF_SECS_ENV)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(MIN_SCREEN_OFF_SECS)
}

/// Relative f2fs node paths (rooted at the sysfs root for tests).
const P_GC_URGENT: &str = "sys/fs/f2fs/sda16/gc_urgent";
const P_GC_SLEEP: &str = "sys/fs/f2fs/sda16/gc_urgent_sleep_time";
const P_DIRTY: &str = "sys/fs/f2fs/sda16/dirty_segments";

/// Persisted maintenance bookkeeping (`maintenance.json` in the state dir).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MaintFile {
    #[serde(default)]
    pub last: u64,
    #[serde(default)]
    pub result: String,
    #[serde(default)]
    pub dirty_before: Option<u64>,
    #[serde(default)]
    pub dirty_after: Option<u64>,
}

impl MaintFile {
    pub fn load(dir: &Path) -> MaintFile {
        fs::read_to_string(dir.join("maintenance.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Atomic write (tmp + rename), like every other state file.
    pub fn save(&self, dir: &Path) {
        let Ok(s) = serde_json::to_string_pretty(self) else {
            return;
        };
        let path = dir.join("maintenance.json");
        let tmp = path.with_extension("json.tmp");
        if fs::write(&tmp, s).is_ok() {
            let _ = fs::rename(&tmp, &path);
        }
    }
}

/// Result of one maintenance run.
#[derive(Debug, Clone, PartialEq)]
pub struct MaintOutcome {
    pub ok: bool,
    pub detail: String,
    pub dirty_before: Option<u64>,
    pub dirty_after: Option<u64>,
}

/// Pure trigger predicate (unit tested): enabled, charging, screen off long
/// enough, and the interval since the last run has passed.
pub fn due(
    enabled: bool,
    charging: Option<bool>,
    screen_on: bool,
    screen_off_secs: u64,
    min_off_secs: u64,
    last_run: u64,
    now: u64,
) -> bool {
    enabled
        && charging == Some(true)
        && !screen_on
        && screen_off_secs >= min_off_secs
        && now.saturating_sub(last_run) >= INTERVAL_SECS
}

pub fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Spawns one run on its own thread; the result arrives as `Msg::MaintDone`.
pub fn spawn(tx: std::sync::mpsc::Sender<super::Msg>) {
    let _ = std::thread::Builder::new()
        .name("maintenance".into())
        .spawn(move || {
            let out = run();
            let _ = tx.send(super::Msg::MaintDone(out));
        });
}

fn node(rel: &str) -> PathBuf {
    default_root().join(rel)
}

fn read_u64(path: &Path) -> Option<u64> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn env_ms(key: &str, fallback: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(fallback)
}

/// One full run on the calling thread. Never panics; every failure is a
/// reported outcome (the daemon must survive storage weirdness).
pub fn run() -> MaintOutcome {
    let dirty_path = node(P_DIRTY);
    let Some(before) = read_u64(&dirty_path) else {
        return MaintOutcome {
            ok: false,
            detail: "f2fs node unavailable".into(),
            dirty_before: None,
            dirty_after: None,
        };
    };
    if before <= DIRTY_THRESHOLD {
        return MaintOutcome {
            ok: true,
            detail: format!("already clean ({before} dirty segments)"),
            dirty_before: Some(before),
            dirty_after: Some(before),
        };
    }

    let sleep_path = node(P_GC_SLEEP);
    let urgent_path = node(P_GC_URGENT);
    let old_sleep = read_u64(&sleep_path);

    if let Err(e) = guarded_write(
        &sleep_path.display().to_string(),
        &URGENT_SLEEP_MS.to_string(),
    ) {
        return MaintOutcome {
            ok: false,
            detail: format!("gc_urgent_sleep_time: {e}"),
            dirty_before: Some(before),
            dirty_after: None,
        };
    }
    if let Err(e) = guarded_write(&urgent_path.display().to_string(), "1") {
        return MaintOutcome {
            ok: false,
            detail: format!("gc_urgent on: {e}"),
            dirty_before: Some(before),
            dirty_after: None,
        };
    }

    let started = Instant::now();
    let max = Duration::from_secs(env_ms(MAX_SECS_ENV, MAX_RUN_SECS));
    let poll = Duration::from_millis(env_ms(POLL_MS_ENV, POLL_MS));
    let mut after = before;
    while started.elapsed() < max {
        std::thread::sleep(poll);
        match read_u64(&dirty_path) {
            Some(v) => {
                after = v;
                if v <= DIRTY_THRESHOLD {
                    break;
                }
            }
            None => break,
        }
    }

    let _ = guarded_write(&urgent_path.display().to_string(), "0");
    if let Some(s) = old_sleep {
        let _ = guarded_write(&sleep_path.display().to_string(), &s.to_string());
    }
    // flush the filesystem after the GC burst (AOSP does the same)
    let _ = std::process::Command::new("/system/bin/sync").output();

    MaintOutcome {
        ok: true,
        detail: format!(
            "gc {before} -> {after} dirty segments in {}s",
            started.elapsed().as_secs()
        ),
        dirty_before: Some(before),
        dirty_after: Some(after),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn due_requires_all_conditions() {
        let now = 1_000_000;
        let last = now - INTERVAL_SECS - 1;
        assert!(due(
            true,
            Some(true),
            false,
            120,
            MIN_SCREEN_OFF_SECS,
            last,
            now
        ));
        // toggle off
        assert!(!due(
            false,
            Some(true),
            false,
            120,
            MIN_SCREEN_OFF_SECS,
            last,
            now
        ));
        // not charging / unknown
        assert!(!due(
            true,
            Some(false),
            false,
            120,
            MIN_SCREEN_OFF_SECS,
            last,
            now
        ));
        assert!(!due(true, None, false, 120, MIN_SCREEN_OFF_SECS, last, now));
        // screen on or not off long enough
        assert!(!due(
            true,
            Some(true),
            true,
            120,
            MIN_SCREEN_OFF_SECS,
            last,
            now
        ));
        assert!(!due(
            true,
            Some(true),
            false,
            10,
            MIN_SCREEN_OFF_SECS,
            last,
            now
        ));
        // too soon
        assert!(!due(
            true,
            Some(true),
            false,
            120,
            MIN_SCREEN_OFF_SECS,
            now - 60,
            now
        ));
    }

    #[test]
    fn maint_file_roundtrip() {
        let dir = crate::daemon::test_util::tmpdir("maint-file");
        let f = MaintFile {
            last: 42,
            result: "gc 500 -> 90".into(),
            dirty_before: Some(500),
            dirty_after: Some(90),
        };
        f.save(&dir);
        let back = MaintFile::load(&dir);
        assert_eq!(back.last, 42);
        assert_eq!(back.dirty_after, Some(90));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_node_is_a_reported_skip() {
        // host has no f2fs nodes; run() must return a clean outcome object
        let out = run();
        assert!(out.dirty_before.is_none());
        assert!(out.detail.contains("unavailable"), "detail: {}", out.detail);
    }
}
