//! Daemon view of `config.json`.
//!
//! Ownership: the APP is the single writer (it also serves the UI when the
//! service is off); the daemon reads — at startup, on an explicit
//! `config_changed` command, and on mtime changes as a fallback.
//!
//! The file is written atomically by the app (tmp + rename) so a torn read
//! is impossible; still, a parse failure keeps the previous config and is
//! reported, never fatal.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub const SCHEMA: u32 = 1;

fn default_true() -> bool {
    true
}

fn default_base() -> String {
    "balance".into()
}

/// User intent mirrored from the app. Unknown fields are ignored (the app
/// may add UI-only keys); missing fields fall back to defaults, so a partial
/// file still yields a sane daemon.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DaemonConfig {
    #[serde(default = "default_one")]
    pub schema: u32,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub dynamic: bool,
    #[serde(default = "default_base")]
    pub base_profile: String,
    #[serde(default)]
    pub app_map: BTreeMap<String, String>,
    #[serde(default = "default_true")]
    pub sync_miui_perf: bool,
    #[serde(default = "default_true")]
    pub sync_miui_saver: bool,
    #[serde(default = "default_true")]
    pub game_mode_checker: bool,
    /// Refresh-rate follow: mapped game -> 120 Hz, powersave app -> 60 Hz.
    #[serde(default = "default_true")]
    pub sync_refresh: bool,
    /// Battery guard: force Power Save below the floor (unless charging).
    #[serde(default = "default_true")]
    pub guard_battery: bool,
    #[serde(default = "default_battery_floor")]
    pub battery_floor_pct: u8,
    /// Thermal guard: step Game down to Balance near the thermal ceiling.
    #[serde(default = "default_true")]
    pub guard_thermal: bool,
    #[serde(default = "default_thermal_ceiling")]
    pub thermal_ceiling_c: f32,
    /// Storage maintenance: weekly bounded f2fs GC while charging + idle.
    #[serde(default)]
    pub maintenance: bool,
    /// Charge guard: pause charging at the limit (opt-in, user-facing switch).
    #[serde(default)]
    pub charge_limit: bool,
    #[serde(default = "default_charge_pct")]
    pub charge_limit_pct: u8,
}

fn default_charge_pct() -> u8 {
    80
}

fn default_battery_floor() -> u8 {
    20
}

fn default_thermal_ceiling() -> f32 {
    75.0
}

fn default_one() -> u32 {
    SCHEMA
}

impl Default for DaemonConfig {
    fn default() -> Self {
        DaemonConfig {
            schema: SCHEMA,
            enabled: true,
            dynamic: true,
            base_profile: default_base(),
            app_map: BTreeMap::new(),
            sync_miui_perf: true,
            sync_miui_saver: true,
            game_mode_checker: true,
            sync_refresh: true,
            guard_battery: true,
            battery_floor_pct: default_battery_floor(),
            guard_thermal: true,
            thermal_ceiling_c: default_thermal_ceiling(),
            maintenance: false,
            charge_limit: false,
            charge_limit_pct: default_charge_pct(),
        }
    }
}

/// Cached config file with change detection.
pub struct ConfigFile {
    path: PathBuf,
    cached: DaemonConfig,
    stamp: Option<SystemTime>,
    len: u64,
}

impl ConfigFile {
    /// Loads the file; on any failure returns defaults + the error reason.
    pub fn load(path: &Path) -> (Self, Option<String>) {
        match fs::read_to_string(path) {
            Ok(raw) => match serde_json::from_str::<DaemonConfig>(&raw) {
                Ok(cfg) => (
                    ConfigFile {
                        path: path.to_path_buf(),
                        cached: cfg,
                        stamp: mtime(path),
                        len: raw.len() as u64,
                    },
                    None,
                ),
                Err(e) => (
                    ConfigFile {
                        path: path.to_path_buf(),
                        cached: default_cfg(),
                        stamp: mtime(path),
                        len: 0,
                    },
                    Some(format!("config parse: {e}")),
                ),
            },
            Err(e) => (
                ConfigFile {
                    path: path.to_path_buf(),
                    cached: default_cfg(),
                    stamp: None,
                    len: 0,
                },
                Some(format!("config read: {e}")),
            ),
        }
    }

    pub fn get(&self) -> &DaemonConfig {
        &self.cached
    }

    /// True (and reload) when the file's mtime/size changed since last read.
    pub fn reload_if_changed(&mut self) -> bool {
        let stamp = mtime(&self.path);
        if stamp == self.stamp && stamp.is_none() {
            return false; // file still absent
        }
        let len = fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0);
        if stamp == self.stamp && len == self.len {
            return false;
        }
        self.reload()
    }

    /// Unconditional reload — used by the explicit `config_changed` hint so
    /// a write can never be missed by mtime granularity. A parse failure
    /// keeps the previous good config (reported, never fatal).
    pub fn reload(&mut self) -> bool {
        let Ok(raw) = fs::read_to_string(&self.path) else {
            return false; // transient (mid-rename) — retry next tick
        };
        match serde_json::from_str::<DaemonConfig>(&raw) {
            Ok(cfg) => {
                self.cached = cfg;
                self.stamp = mtime(&self.path);
                self.len = raw.len() as u64;
                true
            }
            Err(_) => false,
        }
    }
}

fn mtime(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).ok().and_then(|m| m.modified().ok())
}

fn default_cfg() -> DaemonConfig {
    DaemonConfig::default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::test_util::tmpdir;

    fn write(path: &Path, body: &str) {
        fs::write(path, body).unwrap();
    }

    #[test]
    fn defaults_are_service_ready() {
        let c = DaemonConfig::default();
        assert!(c.enabled && c.dynamic);
        assert_eq!(c.base_profile, "balance");
        assert!(c.app_map.is_empty());
        assert!(c.guard_battery && c.guard_thermal);
        assert_eq!(c.battery_floor_pct, 20);
        assert_eq!(c.thermal_ceiling_c, 75.0);
        assert!(c.sync_refresh);
        assert!(!c.maintenance, "maintenance is opt-in");
        assert!(!c.charge_limit, "charge limit is opt-in");
        assert_eq!(c.charge_limit_pct, 80);
    }

    #[test]
    fn partial_json_fills_defaults() {
        let c: DaemonConfig = serde_json::from_str(r#"{"base_profile":"game"}"#).unwrap();
        assert_eq!(c.base_profile, "game");
        assert!(c.enabled, "missing enabled must default to true");
        assert!(c.dynamic);
        assert_eq!(c.schema, SCHEMA);
        assert!(c.guard_battery, "guards default on");
        assert_eq!(c.battery_floor_pct, 20);
        assert_eq!(c.thermal_ceiling_c, 75.0);
    }

    #[test]
    fn guard_fields_roundtrip() {
        let c: DaemonConfig = serde_json::from_str(
            r#"{"guard_battery":false,"battery_floor_pct":15,"guard_thermal":false,"thermal_ceiling_c":80.5}"#,
        )
        .unwrap();
        assert!(!c.guard_battery);
        assert_eq!(c.battery_floor_pct, 15);
        assert!(!c.guard_thermal);
        assert_eq!(c.thermal_ceiling_c, 80.5);
    }

    #[test]
    fn load_missing_file_yields_defaults_and_reason() {
        let dir = tmpdir("cfg-missing");
        let (cfg, err) = ConfigFile::load(&dir.join("config.json"));
        assert!(err.is_some());
        assert!(cfg.get().enabled);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn reload_detects_change_and_keeps_good_on_parse_error() {
        let dir = tmpdir("cfg-reload");
        let path = dir.join("config.json");
        write(&path, r#"{"base_profile":"balance"}"#);
        let (mut cfg, err) = ConfigFile::load(&path);
        assert!(err.is_none());
        assert_eq!(cfg.get().base_profile, "balance");

        // no change -> false
        assert!(!cfg.reload_if_changed());

        // change -> true + new value
        std::thread::sleep(std::time::Duration::from_millis(1100)); // mtime granularity
        write(&path, r#"{"base_profile":"game","dynamic":false}"#);
        assert!(cfg.reload_if_changed());
        assert_eq!(cfg.get().base_profile, "game");
        assert!(!cfg.get().dynamic);

        // broken change -> false, previous value kept
        std::thread::sleep(std::time::Duration::from_millis(1100));
        write(&path, "{broken");
        assert!(!cfg.reload_if_changed());
        assert_eq!(cfg.get().base_profile, "game");
        let _ = fs::remove_dir_all(&dir);
    }
}
