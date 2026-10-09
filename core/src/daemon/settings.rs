//! Live MIUI/Android flags read through the `settings` CLI.
//!
//! The daemon runs as root, so reads/writes go through `/system/bin/settings`
//! (binder-free, verified on device). Reads are polled on a background
//! thread and cached — the decision loop never blocks on exec.
//!
//! `MIFINETUNE_SETTINGS_BIN` overrides the binary path (host E2E tests and
//! simulations point it at a script); the `*_with` helpers take an explicit
//! path so tests never touch process globals.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// AOSP/MIUI battery saver global.
pub const SAVER_KEY: &str = "low_power";
/// MIUI performance switch mirror (the property itself is unreachable).
pub const POWER_MODE_KEY: &str = "power_mode";
/// Legacy F9 key: only used by the one-time migration that gives a held
/// refresh value back after the refresh-follow feature was removed.
pub const REFRESH_KEY: &str = "user_refresh_rate";

const SETTINGS_BIN: &str = "/system/bin/settings";
/// Env override for the settings binary (host tests / simulation).
pub const SETTINGS_BIN_ENV: &str = "MIFINETUNE_SETTINGS_BIN";
const POLL_MS: u64 = 5_000;

/// Resolved settings binary for this process.
pub fn bin_path() -> String {
    std::env::var(SETTINGS_BIN_ENV).unwrap_or_else(|_| SETTINGS_BIN.to_string())
}

/// Cached live flags for the decision loop.
pub struct LiveFlags {
    saver: AtomicBool,
}

impl LiveFlags {
    /// Last polled battery-saver value (user + framework, unattributed).
    pub fn saver(&self) -> bool {
        self.saver.load(Ordering::Relaxed)
    }

    /// Starts the polling thread; lives for the daemon lifetime.
    pub fn spawn() -> Arc<Self> {
        let me = Arc::new(LiveFlags {
            saver: AtomicBool::new(false),
        });
        let keep = me.clone();
        let _ = std::thread::Builder::new()
            .name("live-flags".into())
            .spawn(move || loop {
                if let Some(v) = read_bool("global", SAVER_KEY) {
                    keep.saver.store(v, Ordering::Relaxed);
                }
                std::thread::sleep(Duration::from_millis(POLL_MS));
            });
        me
    }
}

/// `settings get <scope> <key>`; None when the binary or value is missing.
pub fn read(scope: &str, key: &str) -> Option<String> {
    read_with(&bin_path(), scope, key)
}

/// Explicit-binary variant (tests; no process-global state).
pub fn read_with(bin: &str, scope: &str, key: &str) -> Option<String> {
    let out = Command::new(bin).args(["get", scope, key]).output().ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() || s == "null" {
        None
    } else {
        Some(s)
    }
}

pub fn read_bool(scope: &str, key: &str) -> Option<bool> {
    read(scope, key).map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
}

/// `settings put <scope> <key> <value>` (bridge writes).
pub fn put(scope: &str, key: &str, value: &str) -> Result<(), String> {
    put_with(&bin_path(), scope, key, value)
}

/// Explicit-binary variant (tests; no process-global state).
pub fn put_with(bin: &str, scope: &str, key: &str, value: &str) -> Result<(), String> {
    let out = Command::new(bin)
        .args(["put", scope, key, value])
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// `settings delete <scope> <key>` (used when a captured value was absent).
pub fn delete(scope: &str, key: &str) -> Result<(), String> {
    delete_with(&bin_path(), scope, key)
}

pub fn delete_with(bin: &str, scope: &str, key: &str) -> Result<(), String> {
    let out = Command::new(bin)
        .args(["delete", scope, key])
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

// --- refresh-rate FPS switch helper ------------------------------------------
//
// On surya/MIUI-12 the `user_refresh_rate` settings key alone does NOT move
// the panel: the mode is driven by MIUI's display-feature HAL
// (`DisplayFeatureHal::HandleFpsSwitch` -> SDM `SetActiveConfig`). MIUI's own
// RefreshRateActivity calls `DisplayFeatureManager.setScreenEffect(24, hz)`
// (effect id 24, per MiSettings DisplayUtils.java). The call only works from
// a root context, so the daemon runs the tiny embedded dex via
// `app_process` — the exact same API the MIUI UI uses, no props, no SELinux
// changes. Device-verified 2026-10-09: "really set fps(120)" + the active
// mode flips 60 <-> 120.

pub const APP_PROCESS_BIN_ENV: &str = "MIFINETUNE_APP_PROCESS_BIN";
const APP_PROCESS_BIN: &str = "/system/bin/app_process";
pub const REFRESH_DEX_ENV: &str = "MIFINETUNE_REFRESH_DEX";
const REFRESH_DEX_PATH: &str = "/data/local/tmp/mifinetune/refresh_fps.dex";
const REFRESH_DEX_BYTES: &[u8] = include_bytes!("../../assets/refresh_fps.dex");

fn app_process_path() -> String {
    std::env::var(APP_PROCESS_BIN_ENV).unwrap_or_else(|_| APP_PROCESS_BIN.to_string())
}

fn refresh_dex_path() -> PathBuf {
    std::env::var(REFRESH_DEX_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(REFRESH_DEX_PATH))
}

/// Resolves the helper + dex for this process (env overrides for E2E).
pub fn apply_refresh_fps(hz: u32) -> Result<String, String> {
    apply_refresh_fps_with(&app_process_path(), &refresh_dex_path(), hz)
}

/// Explicit-path variant (tests never touch process globals).
pub fn apply_refresh_fps_with(app_bin: &str, dex: &Path, hz: u32) -> Result<String, String> {
    let need_write = fs::read(dex)
        .map(|b| b != REFRESH_DEX_BYTES)
        .unwrap_or(true);
    if need_write {
        if let Some(dir) = dex.parent() {
            fs::create_dir_all(dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
        }
        fs::write(dex, REFRESH_DEX_BYTES).map_err(|e| format!("write dex: {e}"))?;
    }
    let out = Command::new(app_bin)
        .arg("/system/bin")
        .arg("mifinetune.RefreshFps")
        .arg(hz.to_string())
        .env("CLASSPATH", dex)
        .output()
        .map_err(|e| format!("spawn {app_bin}: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        let last = err.lines().last().unwrap_or("app_process failed");
        Err(last.to_string())
    }
}

// --- panel refresh fast path (SurfaceFlinger DFPS transaction) -----------------
//
// `service call SurfaceFlinger 1035 i32 <idx>` switches the panel mode
// instantly. 1035 is MIUI's SURFACE_FLINGER_TRANSACTION_DISPLAY_FEATURE_DFPS
// (ScreenEffectService.java) — the exact transaction MIUI's own framework
// fires when the display-feature HAL reports fps changes. `<idx>` indexes the
// active panel's supported-dfps list from the device tree; for the surya
// nt36672c panels that is `[120, 90, 60, 50, 30]` (idx 0..4).
// Device-verified 2026-10-09/10: idx 0->120, 1->90, 2->60, 3->50, 4->30,
// persistent. Note: the MIUI HAL helper ignores 90 Hz on this build (it
// logs "the setting fps is the same" and no-ops), so the SF transaction is
// the authoritative lever — which is why it runs first.
// This runs *before* the MIUI `setScreenEffect(24)` helper so the visual
// switch is instant while the HAL's own state catches up ~1 s later.
//
// Reverse-engineering notes: docs/ROM-INTERNALS.md (§ display stack).

pub const SERVICE_BIN_ENV: &str = "MIFINETUNE_SERVICE_BIN";
const SERVICE_BIN: &str = "/system/bin/service";
pub const PANEL_INFO_ENV: &str = "MIFINETUNE_PANEL_INFO";
const PANEL_INFO_PATH: &str =
    "/sys/devices/platform/soc/ae00000.qcom,mdss_mdp/drm/card0/card0-DSI-1/panel_info";
pub const DT_BASE_ENV: &str = "MIFINETUNE_DT_BASE";
const DT_BASE: &str = "/sys/firmware/devicetree/base/soc/qcom,mdss_mdp@ae00000";
/// Fallback dfps list for the surya panels (nt36672c huaxing/tianma share it).
const DFPS_FALLBACK: [u32; 5] = [120, 90, 60, 50, 30];

fn service_bin() -> String {
    std::env::var(SERVICE_BIN_ENV).unwrap_or_else(|_| SERVICE_BIN.to_string())
}

fn panel_info_path() -> PathBuf {
    std::env::var(PANEL_INFO_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(PANEL_INFO_PATH))
}

fn dt_base_path() -> PathBuf {
    std::env::var(DT_BASE_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DT_BASE))
}

/// Panel name (e.g. `dsi_nt36672c_huaxing_fhd_video_display`) mapped to the
/// device-tree node that carries the dfps list.
fn dfps_node_dir(panel_name: &str) -> Option<String> {
    let stem = panel_name
        .trim()
        .strip_suffix("_display")?
        .strip_prefix("dsi_")?;
    Some(format!("qcom,mdss_dsi_{stem}"))
}

fn read_dfps_list_from(panel_info: &Path, dt_base: &Path) -> Option<Vec<u32>> {
    let text = fs::read_to_string(panel_info).ok()?;
    let name = text
        .lines()
        .find_map(|l| l.trim().strip_prefix("panel_name="))?;
    let node = dfps_node_dir(name)?;
    let raw = fs::read(dt_base.join(node).join("qcom,dsi-supported-dfps-list")).ok()?;
    if raw.len() < 4 || raw.len() % 4 != 0 {
        return None;
    }
    let list: Vec<u32> = raw
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| u32::from_be_bytes(*c))
        .collect();
    (!list.is_empty()).then_some(list)
}

/// The active panel's dfps list, falling back to the known surya list.
pub fn dfps_list() -> Vec<u32> {
    read_dfps_list_from(&panel_info_path(), &dt_base_path())
        .unwrap_or_else(|| DFPS_FALLBACK.to_vec())
}

/// Position of `hz` inside a dfps list (the SF transaction argument).
pub fn dfps_index(list: &[u32], hz: u32) -> Option<usize> {
    list.iter().position(|&x| x == hz)
}

/// Instant panel switch through the SurfaceFlinger dfps transaction.
pub fn apply_panel_refresh(hz: u32) -> Result<String, String> {
    apply_panel_refresh_with(&service_bin(), &dfps_list(), hz)
}

/// Explicit-path variant (tests never touch process globals).
pub fn apply_panel_refresh_with(service: &str, list: &[u32], hz: u32) -> Result<String, String> {
    let idx =
        dfps_index(list, hz).ok_or_else(|| format!("{hz} Hz not in panel dfps list {list:?}"))?;
    let out = Command::new(service)
        .args(["call", "SurfaceFlinger", "1035", "i32"])
        .arg(idx.to_string())
        .output()
        .map_err(|e| format!("spawn {service}: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        let last = err.lines().last().unwrap_or("service call failed");
        Err(last.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    /// Fake `settings` script: get/put against a state dir (one file per key).
    fn fake_settings(tag: &str) -> (PathBuf, PathBuf) {
        let dir = crate::daemon::test_util::tmpdir(tag);
        let state = dir.join("state");
        fs::create_dir_all(&state).unwrap();
        let script = dir.join("settings");
        fs::write(
            &script,
            format!(
                "#!/bin/sh\ns={}\ncase \"$1\" in\nget) cat \"$s/$2.$3\" 2>/dev/null ;;\nput) printf '%s' \"$4\" > \"$s/$2.$3\" ;;\ndelete) rm -f \"$s/$2.$3\" ;;\nesac\n",
                state.display()
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perm = fs::metadata(&script).unwrap().permissions();
            perm.set_mode(0o755);
            fs::set_permissions(&script, perm).unwrap();
        }
        (script, state)
    }

    #[test]
    fn read_put_roundtrip_through_a_fake_binary() {
        let (script, _state) = fake_settings("settings-rt");
        let bin = script.to_str().unwrap();

        // unset value -> None (script exits 0 with empty output)
        assert_eq!(read_with(bin, "system", "power_mode"), None);

        put_with(bin, "system", "power_mode", "high").unwrap();
        assert_eq!(
            read_with(bin, "system", "power_mode").as_deref(),
            Some("high")
        );

        put_with(bin, "system", "power_mode", "middle").unwrap();
        assert_eq!(
            read_with(bin, "system", "power_mode").as_deref(),
            Some("middle")
        );

        put_with(bin, "global", "low_power", "1").unwrap();
        assert_eq!(read_with(bin, "global", "low_power").as_deref(), Some("1"));
    }

    #[test]
    fn refresh_fps_helper_materializes_dex_and_runs() {
        let dir = crate::daemon::test_util::tmpdir("refresh-fps");
        let log = dir.join("calls.log");
        let bin = dir.join("app_process");
        fs::write(
            &bin,
            format!("#!/bin/sh\necho \"$CLASSPATH $*\" >> {}\n", log.display()),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perm = fs::metadata(&bin).unwrap().permissions();
            perm.set_mode(0o755);
            fs::set_permissions(&bin, perm).unwrap();
        }
        let dex = dir.join("sub").join("refresh.dex");
        apply_refresh_fps_with(bin.to_str().unwrap(), &dex, 90).unwrap();
        assert!(dex.exists(), "the embedded dex must be materialized");
        let calls = fs::read_to_string(&log).unwrap();
        assert!(
            calls.contains("mifinetune.RefreshFps 90"),
            "helper call recorded: {calls}"
        );
        assert!(calls.contains("refresh.dex"), "CLASSPATH points at the dex");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_binary_is_none_not_panic() {
        assert_eq!(
            read_with("/nonexistent/settings-bin", "system", "power_mode"),
            None
        );
        assert!(put_with("/nonexistent/settings-bin", "system", "power_mode", "high").is_err());
    }

    #[test]
    fn dfps_index_maps_hz_to_panel_position() {
        let list = [120, 90, 60, 50, 30];
        assert_eq!(dfps_index(&list, 120), Some(0));
        assert_eq!(dfps_index(&list, 90), Some(1));
        assert_eq!(dfps_index(&list, 60), Some(2));
        assert_eq!(dfps_index(&list, 50), Some(3));
        assert_eq!(dfps_index(&list, 30), Some(4));
        assert_eq!(dfps_index(&list, 144), None);
    }

    #[test]
    fn dfps_list_reads_the_device_tree() {
        let dir = crate::daemon::test_util::tmpdir("panel-dt");
        fs::write(
            dir.join("panel_info"),
            "panel_name=dsi_nt36672c_huaxing_fhd_video_display\n",
        )
        .unwrap();
        let node = dir
            .join("dt")
            .join("qcom,mdss_mdp@ae00000")
            .join("qcom,mdss_dsi_nt36672c_huaxing_fhd_video");
        fs::create_dir_all(&node).unwrap();
        let mut bytes = Vec::new();
        for v in [120u32, 90, 60, 50, 30] {
            bytes.extend_from_slice(&v.to_be_bytes());
        }
        fs::write(node.join("qcom,dsi-supported-dfps-list"), bytes).unwrap();
        let list = read_dfps_list_from(
            &dir.join("panel_info"),
            &dir.join("dt").join("qcom,mdss_mdp@ae00000"),
        )
        .unwrap();
        assert_eq!(list, vec![120, 90, 60, 50, 30]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn panel_refresh_runs_service_call() {
        let dir = crate::daemon::test_util::tmpdir("panel-call");
        let log = dir.join("calls.log");
        let bin = dir.join("service");
        fs::write(
            &bin,
            format!("#!/bin/sh\necho \"$*\" >> {}\n", log.display()),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perm = fs::metadata(&bin).unwrap().permissions();
            perm.set_mode(0o755);
            fs::set_permissions(&bin, perm).unwrap();
        }
        apply_panel_refresh_with(bin.to_str().unwrap(), &[120, 90, 60, 50, 30], 60).unwrap();
        let calls = fs::read_to_string(&log).unwrap();
        assert!(
            calls.contains("call SurfaceFlinger 1035 i32 2"),
            "60 Hz must map to dfps index 2: {calls}"
        );
        assert!(
            apply_panel_refresh_with(bin.to_str().unwrap(), &[120, 90, 60], 30).is_err(),
            "unsupported rates must be reported, not silently ignored"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
