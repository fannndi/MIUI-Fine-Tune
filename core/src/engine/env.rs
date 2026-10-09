//! Read-only environment telemetry: battery, thermal zones, GPU busy.
//!
//! Everything in this file is READ-ONLY — no writes, no framework fights.
//! The daemon uses it for diagnostics (UI) and env-aware guards; `doctor`
//! uses it to verify the device exposes what the daemon needs.
//!
//! Paths are rooted at a configurable prefix (`MIFINETUNE_SYSFS_ROOT`,
//! default `/`) so host tests and simulations can point the sampler at a
//! fake tree — the same trick `MIFINETUNE_LOGCAT_BIN` uses for logcat.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Env override for the sysfs prefix (host tests / simulation).
pub const SYSFS_ROOT_ENV: &str = "MIFINETUNE_SYSFS_ROOT";

/// Preferred thermal zone types, in priority order (surya / sm6150 names).
const CPU_ZONES: &[&str] = &["cpuss-0-usr", "cpu-0-0-usr", "pm6150-tz"];
const GPU_ZONES: &[&str] = &["gpuss-0-usr", "gpuss-1-usr"];

/// One telemetry sample. Every field is optional: a missing node must never
/// break the daemon — the field is simply absent.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct EnvSnapshot {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub battery_pct: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub charging: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub battery_temp_c: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_temp_c: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu_temp_c: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu_busy_pct: Option<u8>,
    /// Panel frame rate from the read-only `measured_fps` node (DRM CRTC).
    /// Read-only telemetry: MiFineTune never writes it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub screen_fps: Option<f32>,
}

/// The sysfs prefix for this process (`/` on device, a fixture in tests).
pub fn default_root() -> PathBuf {
    std::env::var_os(SYSFS_ROOT_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// Battery + thermal + GPU sampler (zone paths resolved once at startup).
pub struct Sampler {
    root: PathBuf,
    cpu_temp: Option<PathBuf>,
    gpu_temp: Option<PathBuf>,
}

impl Sampler {
    pub fn new(root: &Path) -> Self {
        let zones = discover_zones(root);
        let pick = |names: &[&str]| {
            names
                .iter()
                .find_map(|n| zones.iter().find(|(t, _)| t == n).map(|(_, p)| p.clone()))
        };
        Sampler {
            root: root.to_path_buf(),
            cpu_temp: pick(CPU_ZONES),
            gpu_temp: pick(GPU_ZONES),
        }
    }

    pub fn sample(&self) -> EnvSnapshot {
        EnvSnapshot {
            battery_pct: self.read_parse("sys/class/power_supply/battery/capacity", parse_pct),
            charging: self.read_parse("sys/class/power_supply/battery/status", parse_charging),
            battery_temp_c: self
                .read_parse("sys/class/power_supply/battery/temp", parse_temp_tenths),
            cpu_temp_c: self
                .cpu_temp
                .as_deref()
                .and_then(read_trim)
                .and_then(|s| parse_temp_milli(&s)),
            gpu_temp_c: self
                .gpu_temp
                .as_deref()
                .and_then(read_trim)
                .and_then(|s| parse_temp_milli(&s)),
            gpu_busy_pct: self.read_parse(
                "sys/class/kgsl/kgsl-3d0/gpu_busy_percentage",
                parse_gpu_busy,
            ),
            screen_fps: self
                .read_parse("sys/class/drm/sde-crtc-0/measured_fps", parse_measured_fps),
        }
    }

    fn read_parse<T>(&self, rel: &str, f: fn(&str) -> Option<T>) -> Option<T> {
        read_trim(&self.root.join(rel)).and_then(|s| f(&s))
    }
}

/// `(type, temp-path)` for every `thermal_zone*` under the root.
fn discover_zones(root: &Path) -> Vec<(String, PathBuf)> {
    let base = root.join("sys/class/thermal");
    let mut zones = Vec::new();
    let Ok(rd) = fs::read_dir(&base) else {
        return zones;
    };
    for e in rd.flatten() {
        let p = e.path();
        let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.starts_with("thermal_zone") {
            continue;
        }
        if let Some(t) = read_trim(&p.join("type")) {
            zones.push((t, p.join("temp")));
        }
    }
    zones
}

fn read_trim(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

// --- pure parsers (unit tested without a filesystem) -------------------------

/// "95" -> Some(95), clamped to 0..=100; non-numeric -> None.
pub fn parse_pct(raw: &str) -> Option<u8> {
    raw.trim()
        .parse::<i64>()
        .ok()
        .map(|v| v.clamp(0, 100) as u8)
}

/// power_supply battery temp: tenths of °C ("320" -> 32.0). <= 0 -> None.
pub fn parse_temp_tenths(raw: &str) -> Option<f32> {
    let v = raw.trim().parse::<i64>().ok()?;
    if v <= 0 {
        return None;
    }
    Some(v as f32 / 10.0)
}

/// thermal_zone temp: millidegrees ("38800" -> 38.8). <= 0 -> None
/// (level-indicator zones use negative numbers; we never sample those).
pub fn parse_temp_milli(raw: &str) -> Option<f32> {
    let v = raw.trim().parse::<i64>().ok()?;
    if v <= 0 {
        return None;
    }
    Some(v as f32 / 1000.0)
}

/// power_supply status -> charging? "Charging"/"Full" true, the rest false;
/// unknown strings -> None (absent is honest).
pub fn parse_charging(raw: &str) -> Option<bool> {
    match raw.trim() {
        "Charging" | "Full" => Some(true),
        "Discharging" | "Not charging" => Some(false),
        _ => None,
    }
}

/// "3 %" or "3" -> Some(3), clamped to 0..=100.
pub fn parse_gpu_busy(raw: &str) -> Option<u8> {
    let first = raw.split_whitespace().next()?;
    first.parse::<i64>().ok().map(|v| v.clamp(0, 100) as u8)
}

/// DRM `measured_fps` -> frames per second:
/// "fps: 75.1 duration:1000000 frame_count:97" -> Some(75.1).
/// Screen off reports fps: 0.0 -> Some(0.0) (honest zero, not absent).
pub fn parse_measured_fps(raw: &str) -> Option<f32> {
    let rest = raw.trim().strip_prefix("fps:")?;
    rest.split_whitespace().next()?.parse::<f32>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(tag: &str) -> PathBuf {
        let mut d = std::env::temp_dir();
        d.push(format!("mifinetune-env-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn fake_tree(tag: &str) -> PathBuf {
        let root = tmpdir(tag);
        let w = |rel: &str, body: &str| {
            let p = root.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, body).unwrap();
        };
        w("sys/class/power_supply/battery/capacity", "95\n");
        w("sys/class/power_supply/battery/status", "Charging\n");
        w("sys/class/power_supply/battery/temp", "320\n");
        w("sys/class/thermal/thermal_zone0/type", "pm6150-tz\n");
        w("sys/class/thermal/thermal_zone0/temp", "37000\n");
        w("sys/class/thermal/thermal_zone24/type", "cpuss-0-usr\n");
        w("sys/class/thermal/thermal_zone24/temp", "38800\n");
        w("sys/class/thermal/thermal_zone30/type", "gpuss-0-usr\n");
        w("sys/class/thermal/thermal_zone30/temp", "41200\n");
        w("sys/class/kgsl/kgsl-3d0/gpu_busy_percentage", "3 %\n");
        w(
            "sys/class/drm/sde-crtc-0/measured_fps",
            "fps: 75.1 duration:1000000 frame_count:97\n",
        );
        root
    }

    #[test]
    fn parsers_accept_device_formats() {
        assert_eq!(parse_pct("95\n"), Some(95));
        assert_eq!(parse_pct("101"), Some(100));
        assert_eq!(parse_pct("-5"), Some(0));
        assert_eq!(parse_pct("abc"), None);

        assert_eq!(parse_temp_tenths("320"), Some(32.0));
        assert_eq!(parse_temp_tenths("0"), None);
        assert_eq!(parse_temp_tenths("-100"), None);

        assert_eq!(parse_temp_milli("38800"), Some(38.8));
        assert_eq!(
            parse_temp_milli("-312"),
            None,
            "level zones are not temperatures"
        );

        assert_eq!(parse_charging("Charging"), Some(true));
        assert_eq!(parse_charging("Full"), Some(true));
        assert_eq!(parse_charging("Discharging"), Some(false));
        assert_eq!(parse_charging("Not charging"), Some(false));
        assert_eq!(parse_charging("Unknown"), None);

        assert_eq!(parse_gpu_busy("3 %"), Some(3));
        assert_eq!(parse_gpu_busy("100"), Some(100));
        assert_eq!(parse_gpu_busy("bogus"), None);

        assert_eq!(
            parse_measured_fps("fps: 75.1 duration:1000000 frame_count:97"),
            Some(75.1)
        );
        assert_eq!(parse_measured_fps("fps: 0.0 duration:1000000"), Some(0.0));
        assert_eq!(parse_measured_fps("garbage"), None);
    }

    #[test]
    fn sampler_reads_a_fake_tree() {
        let root = fake_tree("sample");
        let s = Sampler::new(&root).sample();
        assert_eq!(s.battery_pct, Some(95));
        assert_eq!(s.charging, Some(true));
        assert_eq!(s.battery_temp_c, Some(32.0));
        assert_eq!(
            s.cpu_temp_c,
            Some(38.8),
            "cpuss-0-usr must win over pm6150-tz"
        );
        assert_eq!(s.gpu_temp_c, Some(41.2));
        assert_eq!(s.gpu_busy_pct, Some(3));
        assert_eq!(s.screen_fps, Some(75.1));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_nodes_yield_none_not_panic() {
        let root = tmpdir("empty");
        let s = Sampler::new(&root).sample();
        assert_eq!(s, EnvSnapshot::default());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn zone_discovery_falls_back_in_priority_order() {
        let root = tmpdir("zones");
        let w = |rel: &str, body: &str| {
            let p = root.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, body).unwrap();
        };
        w("sys/class/thermal/thermal_zone0/type", "pm6150-tz\n");
        w("sys/class/thermal/thermal_zone0/temp", "37000\n");
        w("sys/class/thermal/thermal_zone7/type", "cpu-0-0-usr\n");
        w("sys/class/thermal/thermal_zone7/temp", "39900\n");
        let s = Sampler::new(&root).sample();
        assert_eq!(s.cpu_temp_c, Some(39.9), "cpu-0-0-usr beats pm6150-tz");
        assert_eq!(s.gpu_temp_c, None, "no gpu zone -> absent");
        let _ = fs::remove_dir_all(&root);
    }
}
