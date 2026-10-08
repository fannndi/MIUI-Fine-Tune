//! Per-kind value validation against live probe options.
//!
//! Responsibility: `(resolved, None)` when acceptable (clamped to real OPPs /
//! core_ctl bounds), `(raw, Some(reason))` when locked.
//! Non-goals: plan/status (`plan.rs`), read-back compare (`readback.rs`).

use crate::engine::catalog::{self, Kind};
use crate::engine::probe::ProbeData;

/// Validate `wanted` for an entry against live options.
/// Returns `(resolved, None)` or `(raw, Some(reason))` when locked.
pub fn validate_value(e: &catalog::Entry, wanted: &str, probe: &ProbeData) -> (String, Option<String>) {
    let wanted = wanted.trim();
    let locked = |msg: &str| (wanted.to_string(), Some(msg.to_string()));
    match e.kind {
        Kind::Int | Kind::RepeatInt => match wanted.parse::<i64>() {
            Ok(v) if v >= 0 => {
                if let Some(scope) = e.min_cpus_of {
                    // kernel: if (val < state->num_cpus) return -EINVAL
                    let n = probe.options.cluster_cpus.get(scope).copied().unwrap_or(0);
                    if n > 0 && (v as usize) < n {
                        return locked(&format!("must be >= {n} (cluster CPU count, core_ctl.c)"));
                    }
                }
                if let Some((lo, hi)) = e.range {
                    if v < lo || v > hi {
                        return locked(&format!("out of range {lo}..={hi} (kernel bound)"));
                    }
                }
                (v.to_string(), None)
            }
            _ => locked("not a non-negative integer"),
        },
        Kind::MinCpus => match wanted.parse::<i64>() {
            Ok(v) if v >= 0 => {
                // kernel clamps silently: min(val, max_cpus) — pre-clamp so
                // read-back verification matches instead of flagging drift.
                let max = probe.options.core_ctl_max.get(e.scope).copied();
                match max {
                    Some(m) => (v.min(m).to_string(), None),
                    None => (v.to_string(), None),
                }
            }
            _ => locked("not a non-negative integer"),
        },
        Kind::Freq | Kind::FreqMin | Kind::FreqMax => {
            let n: u64 = match wanted.parse() {
                Ok(n) => n,
                Err(_) => return locked("not a frequency"),
            };
            match probe.options.freqs.get(e.scope) {
                Some(list) if !list.is_empty() => {
                    let best = list.iter().min_by_key(|f| (**f as i64 - n as i64).unsigned_abs());
                    (best.unwrap().to_string(), None)
                }
                _ => locked("no OPP list for scope"),
            }
        }
        Kind::PwrLevel => match wanted.parse::<usize>() {
            Ok(v) if probe.options.gpu_levels > 0 && v < probe.options.gpu_levels => (v.to_string(), None),
            _ => locked(&format!(
                "pwrlevel out of range 0..{}",
                probe.options.gpu_levels.saturating_sub(1)
            )),
        },
        Kind::Gov => one_of(wanted, probe.options.governors.values().next().map(|v| v.as_slice()), "governor"),
        Kind::GpuGov => one_of(wanted, Some(&probe.options.gpu_governors), "gpu governor"),
        Kind::IoSched => one_of(wanted, Some(&probe.options.io_schedulers), "io scheduler"),
        Kind::TcpCc => one_of(wanted, Some(&probe.options.tcp_cc), "tcp cc"),
        Kind::Ints => {
            let toks: Vec<&str> = wanted.split_whitespace().collect();
            if toks.is_empty() || !toks.iter().all(|t| t.parse::<i64>().map(|v| v >= 0).unwrap_or(false)) {
                return locked("not a list of non-negative integers");
            }
            (toks.join(" "), None)
        }
        Kind::Mask => match validate_mask(wanted, probe.options.cpu_count) {
            Ok(norm) => (norm, None),
            Err(msg) => locked(&msg),
        },
        Kind::FlagYN => match wanted.to_ascii_uppercase().as_str() {
            "Y" | "N" | "0" | "1" => (wanted.to_ascii_uppercase(), None),
            _ => locked("expected Y/N/0/1"),
        },
        Kind::Text => {
            if wanted.is_empty() {
                locked("empty value")
            } else {
                (wanted.to_string(), None)
            }
        }
    }
}

fn one_of(wanted: &str, options: Option<&[String]>, what: &str) -> (String, Option<String>) {
    match options {
        Some(opts) if opts.iter().any(|o| o == wanted) => (wanted.to_string(), None),
        Some(opts) => (
            wanted.to_string(),
            Some(format!("{what} not available (have: {})", opts.join(" "))),
        ),
        None => (wanted.to_string(), Some(format!("{what} options unreadable"))),
    }
}

/// Validate a CPU mask (`0-5`, `0,2,4-7`) against the online CPU count.
pub fn validate_mask(want: &str, cpu_count: usize) -> Result<String, String> {
    let mut parts = Vec::new();
    for tok in want.split(',') {
        let tok = tok.trim();
        if tok.is_empty() {
            return Err("empty mask token".into());
        }
        let segs: Vec<&str> = tok.split('-').collect();
        let (lo, hi) = match segs.as_slice() {
            [a] => {
                let v: usize = a.parse().map_err(|_| format!("bad cpu '{a}'"))?;
                (v, v)
            }
            [a, b] => {
                let x: usize = a.parse().map_err(|_| format!("bad cpu '{a}'"))?;
                let y: usize = b.parse().map_err(|_| format!("bad cpu '{b}'"))?;
                if x > y {
                    return Err(format!("inverted range {tok}"));
                }
                (x, y)
            }
            _ => return Err(format!("bad mask token '{tok}'")),
        };
        if hi >= cpu_count {
            return Err(format!("cpu {hi} >= cpu_count {cpu_count}"));
        }
        parts.push(if lo == hi { lo.to_string() } else { format!("{lo}-{hi}") });
    }
    Ok(parts.join(","))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_validated_against_cpu_count() {
        assert!(validate_mask("0-7", 8).is_ok());
        assert!(validate_mask("0-8", 8).is_err());
        assert!(validate_mask("0,2,4-7", 8).is_ok());
        assert!(validate_mask("7-0", 8).is_err());
        assert!(validate_mask("a", 8).is_err());
    }
}
