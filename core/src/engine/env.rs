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
    /// Battery current in µA (read-only FG node): negative = charging,
    /// positive = discharging. Sign verified 2026-10-09 with a controlled
    /// `input_suspend` bypass test (suspend: +600 mA; restore: negative).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub battery_current_ua: Option<i64>,
    /// Battery voltage in µV (~4 100 000 near full charge).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub battery_voltage_uv: Option<u32>,
    /// Little/policy0 and big/policy6 current CPU frequency in MHz
    /// (`cpuinfo_cur_freq` is kHz and 0400 root-only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub little_freq_mhz: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub big_freq_mhz: Option<u32>,
    /// GPU current clock in MHz (`gpuclk` reports Hz on this kernel).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu_freq_mhz: Option<u32>,
    /// F2FS userdata lifetime write counter in KB (read-only, updates lazily).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storage_written_kb: Option<u64>,
    /// Battery full-charge capacity in mAh (`charge_full` reports µAh) —
    /// battery-health readout; a broken/negative node stays absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub charge_full_mah: Option<u32>,
    /// Battery cycle count (`cycle_count`; absent on some fuel gauges).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cycle_count: Option<u32>,
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
            battery_current_ua: self
                .read_parse("sys/class/power_supply/battery/current_now", first_i64),
            battery_voltage_uv: self
                .read_parse("sys/class/power_supply/battery/voltage_now", first_u32),
            little_freq_mhz: self.read_parse(
                "sys/devices/system/cpu/cpufreq/policy0/cpuinfo_cur_freq",
                khz_to_mhz,
            ),
            big_freq_mhz: self.read_parse(
                "sys/devices/system/cpu/cpufreq/policy6/cpuinfo_cur_freq",
                khz_to_mhz,
            ),
            gpu_freq_mhz: self.read_parse("sys/class/kgsl/kgsl-3d0/gpuclk", hz_to_mhz),
            storage_written_kb: self
                .read_parse("sys/fs/f2fs/sda16/lifetime_write_kbytes", first_u64),
            charge_full_mah: self
                .read_parse("sys/class/power_supply/battery/charge_full", uah_to_mah),
            cycle_count: self.read_parse("sys/class/power_supply/battery/cycle_count", first_u32),
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

/// First whitespace token as signed integer (battery current in µA).
pub fn first_i64(raw: &str) -> Option<i64> {
    raw.split_whitespace().next()?.parse::<i64>().ok()
}

/// First whitespace token as u32.
pub fn first_u32(raw: &str) -> Option<u32> {
    raw.split_whitespace().next()?.parse::<u32>().ok()
}

/// First whitespace token as u64.
pub fn first_u64(raw: &str) -> Option<u64> {
    raw.split_whitespace().next()?.parse::<u64>().ok()
}

/// `charge_full` reports µAh; diagnostics speak mAh ("5008000" -> 5008).
/// Non-positive values (broken fuel gauge) stay absent.
pub fn uah_to_mah(raw: &str) -> Option<u32> {
    let v = first_i64(raw)?;
    if v <= 0 {
        None
    } else {
        Some((v / 1000) as u32)
    }
}

/// `cpuinfo_cur_freq` reports kHz; diagnostics speak MHz ("1804800" -> 1804).
pub fn khz_to_mhz(raw: &str) -> Option<u32> {
    first_i64(raw).map(|v| (v / 1000) as u32)
}

/// kgsl `gpuclk` reports Hz; diagnostics speak MHz ("430000000" -> 430).
pub fn hz_to_mhz(raw: &str) -> Option<u32> {
    first_i64(raw).map(|v| (v / 1_000_000) as u32)
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
        w("sys/class/power_supply/battery/current_now", "-580123\n");
        w("sys/class/power_supply/battery/voltage_now", "3985000\n");
        w(
            "sys/devices/system/cpu/cpufreq/policy0/cpuinfo_cur_freq",
            "1804800\n",
        );
        w(
            "sys/devices/system/cpu/cpufreq/policy6/cpuinfo_cur_freq",
            "2304000\n",
        );
        w("sys/class/kgsl/kgsl-3d0/gpuclk", "430000000\n");
        w("sys/fs/f2fs/sda16/lifetime_write_kbytes", "53193232\n");
        w("sys/class/power_supply/battery/charge_full", "5008000\n");
        w("sys/class/power_supply/battery/cycle_count", "562\n");
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

        assert_eq!(first_i64(" -121765\n"), Some(-121765));
        assert_eq!(first_i64("garbage"), None);
        assert_eq!(first_u32("4105283\n"), Some(4105283));
        assert_eq!(first_u64("53193232\n"), Some(53193232));
        assert_eq!(uah_to_mah("5008000\n"), Some(5008));
        assert_eq!(uah_to_mah("-203568155"), None, "broken FG stays absent");
        assert_eq!(uah_to_mah("0"), None);
        assert_eq!(uah_to_mah("garbage"), None);
        assert_eq!(khz_to_mhz("1804800\n"), Some(1804));
        assert_eq!(khz_to_mhz("768000"), Some(768));
        assert_eq!(hz_to_mhz("430000000\n"), Some(430));
        assert_eq!(hz_to_mhz("garbage"), None);
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
        assert_eq!(s.battery_current_ua, Some(-580123));
        assert_eq!(s.battery_voltage_uv, Some(3985000));
        assert_eq!(s.little_freq_mhz, Some(1804));
        assert_eq!(s.big_freq_mhz, Some(2304));
        assert_eq!(s.gpu_freq_mhz, Some(430));
        assert_eq!(s.storage_written_kb, Some(53193232));
        assert_eq!(s.charge_full_mah, Some(5008));
        assert_eq!(s.cycle_count, Some(562));
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
