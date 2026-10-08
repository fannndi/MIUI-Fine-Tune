//! Transition history: bounded, persisted record of applied profile changes.
//!
//! One entry per real switch (profile or stock change), capped at
//! [`MAX_ENTRIES`], persisted atomically to `stats.json` in the state dir.
//! The app fetches it with the `stats` command (dashboard + diagnostics);
//! the daemon never interprets the history itself — it is display data.
//!
//! Timestamps are wall-clock epoch seconds because the history is a *human*
//! timeline; all decision timing stays on monotonic `Instant` elsewhere.

use crate::engine::env::EnvSnapshot;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const SCHEMA: u32 = 1;
/// Ring capacity: at 20+ switches/day this covers weeks.
pub const MAX_ENTRIES: usize = 500;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StatEntry {
    /// Epoch seconds (display only).
    pub t: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// Profile id, or "stock" for a restore back to factory values.
    pub to: String,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub battery: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temp_c: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatsFile {
    #[serde(default = "default_schema")]
    pub schema: u32,
    #[serde(default)]
    pub entries: Vec<StatEntry>,
}

fn default_schema() -> u32 {
    SCHEMA
}

impl Default for StatsFile {
    fn default() -> Self {
        StatsFile { schema: SCHEMA, entries: Vec::new() }
    }
}

/// In-memory history + its backing file. Owned by the main loop.
pub struct Stats {
    path: PathBuf,
    pub entries: Vec<StatEntry>,
}

impl Stats {
    /// Loads `stats.json`; a missing/corrupt file starts empty (never fatal).
    pub fn load(state_dir: &Path) -> Stats {
        let path = state_dir.join("stats.json");
        let entries = fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<StatsFile>(&s).ok())
            .map(|f| f.entries)
            .unwrap_or_default();
        let mut stats = Stats { path, entries };
        stats.cap();
        stats
    }

    /// Appends one switch and persists immediately (switches are rare; a
    /// crash must lose at most the in-flight entry).
    pub fn record(&mut self, from: Option<&str>, to: &str, reason: &str, env: &EnvSnapshot) {
        self.entries.push(StatEntry {
            t: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            from: from.map(str::to_string),
            to: to.to_string(),
            reason: reason.to_string(),
            battery: env.battery_pct,
            temp_c: env.cpu_temp_c,
        });
        self.cap();
        self.persist();
    }

    fn cap(&mut self) {
        if self.entries.len() > MAX_ENTRIES {
            let drop = self.entries.len() - MAX_ENTRIES;
            self.entries.drain(0..drop);
        }
    }

    /// Atomic write (tmp + rename), like every other state file.
    fn persist(&self) {
        let file = StatsFile { schema: SCHEMA, entries: self.entries.clone() };
        let Ok(s) = serde_json::to_string(&file) else { return };
        let tmp = self.path.with_extension("json.tmp");
        if fs::write(&tmp, s).is_ok() {
            let _ = fs::rename(&tmp, &self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env() -> EnvSnapshot {
        EnvSnapshot { battery_pct: Some(85), cpu_temp_c: Some(38.5), ..Default::default() }
    }

    #[test]
    fn record_appends_with_env_context() {
        let dir = crate::daemon::test_util::tmpdir("stats-rec");
        let mut s = Stats::load(&dir);
        s.record(None, "balance", "base", &env());
        s.record(Some("balance"), "game", "app", &env());
        assert_eq!(s.entries.len(), 2);
        assert_eq!(s.entries[0].from, None);
        assert_eq!(s.entries[1].from.as_deref(), Some("balance"));
        assert_eq!(s.entries[1].to, "game");
        assert_eq!(s.entries[1].battery, Some(85));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn history_survives_a_reload() {
        let dir = crate::daemon::test_util::tmpdir("stats-reload");
        {
            let mut s = Stats::load(&dir);
            s.record(None, "game", "app", &env());
        }
        let s = Stats::load(&dir);
        assert_eq!(s.entries.len(), 1);
        assert_eq!(s.entries[0].to, "game");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn ring_caps_at_max_keeping_newest() {
        let dir = crate::daemon::test_util::tmpdir("stats-cap");
        let mut s = Stats::load(&dir);
        for i in 0..(MAX_ENTRIES + 10) {
            s.record(None, &format!("p{i}"), "base", &EnvSnapshot::default());
        }
        assert_eq!(s.entries.len(), MAX_ENTRIES);
        assert_eq!(s.entries.first().unwrap().to, format!("p{}", 10));
        assert_eq!(s.entries.last().unwrap().to, format!("p{}", MAX_ENTRIES + 9));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_file_starts_empty_not_panic() {
        let dir = crate::daemon::test_util::tmpdir("stats-corrupt");
        fs::write(dir.join("stats.json"), "{broken").unwrap();
        let s = Stats::load(&dir);
        assert!(s.entries.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }
}
