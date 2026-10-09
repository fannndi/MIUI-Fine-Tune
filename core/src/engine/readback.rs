//! Read-back comparison + snapshot normalization (harmony rules).
//!
//! Responsibility: decide whether a live value matches the wanted one, and
//! normalize live values for the stock snapshot.
//! Non-goals: IO, planning.

use crate::engine::catalog::Kind;

/// Normalize a read-back value for comparison against `resolved`.
/// Strict variant used by the reconcile pass (`force` applies): the
/// freq-family external-QoS shortcuts are deliberately bypassed so a
/// leftover value (from a crashed apply or a stale session) gets rewritten,
/// not just accepted as "framework winning".
pub fn readback_matches_exact(kind: Kind, resolved: &str, rb: &str) -> bool {
    match kind {
        Kind::Freq | Kind::FreqMin | Kind::FreqMax => rb.trim() == resolved,
        _ => readback_matches(kind, resolved, rb),
    }
}

pub fn readback_matches(kind: Kind, resolved: &str, readback: &str) -> bool {
    let rb = readback.trim();
    match kind {
        Kind::Ints => {
            let a: Vec<&str> = resolved.split_whitespace().collect();
            let b: Vec<&str> = rb.split_whitespace().collect();
            a == b
        }
        Kind::IoSched => {
            // read-back is the offered list with the ACTIVE one bracketed:
            // "noop deadline [cfq]" — only the bracketed token counts. A bare
            // token match would flag any offered-but-inactive scheduler as
            // in-sync (real bug caught 2026-10-07: an apply of
            // io.scheduler=deadline while cfq was active reported Unchanged
            // and never switched the elevator).
            let active = rb
                .split_whitespace()
                .find(|t| t.starts_with('[') && t.ends_with(']'))
                .map(|t| t.trim_matches(|c| c == '[' || c == ']'))
                .unwrap_or(rb);
            active == resolved
        }
        Kind::RepeatInt => {
            // kernel expands one int to per-cpu array: all must equal wanted.
            // A stock capture may legitimately store the full array (uniform
            // busy_*_thres) — compare element-wise in that case.
            let want: Vec<i64> = match resolved
                .split_whitespace()
                .map(|t| t.parse::<i64>())
                .collect::<Result<Vec<_>, _>>()
            {
                Ok(v) if !v.is_empty() => v,
                _ => return false,
            };
            let vals: Option<Vec<i64>> = rb.split_whitespace().map(|t| t.parse().ok()).collect();
            match vals {
                Some(v) if !v.is_empty() => {
                    if want.len() == 1 {
                        v.iter().all(|x| *x == want[0])
                    } else {
                        v == want
                    }
                }
                _ => false,
            }
        }
        Kind::FlagYN => {
            rb.eq_ignore_ascii_case(resolved)
                || matches!(
                    (rb, resolved),
                    ("0", "N") | ("N", "0") | ("1", "Y") | ("Y", "1")
                )
        }
        Kind::FreqMax => {
            // A stricter external cap (thermal cooling / freq-QoS) below the
            // requested cap is thermal winning, not drift. Only live > want
            // (cap lost) must be corrected.
            match (resolved.parse::<i64>(), rb.parse::<i64>()) {
                (Ok(want), Ok(live)) => live <= want,
                _ => rb == resolved,
            }
        }
        Kind::FreqMin => {
            // An external floor (perf HAL msm_performance cpu_min_freq QoS)
            // above the requested floor is the framework winning, not drift.
            // Kernel mechanism: msm_performance.c perf_adjust_notify ->
            // CPUFREQ_ADJUST -> cpufreq_verify_within_limits clamps
            // policy->min before the store; scaling_min_freq then reports the
            // clamped value. Verified live 2026-10-07: with QoS min 1248000
            // active, writing 576000 read back 1248000, and after the QoS was
            // removed the node returned to the written value by itself.
            // Only live < want (floor lost) must be corrected.
            match (resolved.parse::<i64>(), rb.parse::<i64>()) {
                (Ok(want), Ok(live)) => live >= want,
                _ => rb == resolved,
            }
        }
        _ => rb == resolved,
    }
}

/// Normalize a live value before it is stored in the stock snapshot, so the
/// restore path writes something the node accepts (scheduler active token,
/// whitespace-normalized int lists, ...).
pub fn normalize_snapshot(kind: Kind, raw: &str) -> String {
    let raw = raw.trim();
    match kind {
        Kind::IoSched => {
            // prefer the bracketed active token; else the whole string
            for tok in raw.split_whitespace() {
                if tok.starts_with('[') && tok.ends_with(']') {
                    return tok.trim_matches(|c| c == '[' || c == ']').to_string();
                }
            }
            raw.to_string()
        }
        Kind::Ints => raw.split_whitespace().collect::<Vec<_>>().join(" "),
        Kind::RepeatInt => {
            // Uniform arrays (the only shape the kernel broadcasts) collapse
            // to a single value so merged-stock ops re-validate and verify.
            // A non-uniform array stays verbatim (element-wise compare).
            let toks: Vec<&str> = raw.split_whitespace().collect();
            let nums: Result<Vec<i64>, _> = toks.iter().map(|t| t.parse::<i64>()).collect();
            match nums {
                Ok(v) if !v.is_empty() && v.iter().all(|x| *x == v[0]) => v[0].to_string(),
                _ => toks.join(" "),
            }
        }
        Kind::Mask => raw.split_whitespace().collect::<Vec<_>>().join(","),
        Kind::FlagYN => {
            let up = raw.to_ascii_uppercase();
            match up.as_str() {
                "Y" | "1" => "Y".into(),
                "N" | "0" => "N".into(),
                other => other.to_string(),
            }
        }
        _ => raw.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeat_int_readback_all_equal() {
        assert!(readback_matches(Kind::RepeatInt, "60", "60 60 60 60 60 60"));
        assert!(!readback_matches(
            Kind::RepeatInt,
            "60",
            "60 40 60 60 60 60"
        ));
        // a full-array resolved value compares element-wise (stock capture)
        assert!(readback_matches(
            Kind::RepeatInt,
            "60 60 60 60 60 60",
            "60 60 60 60 60 60"
        ));
        assert!(!readback_matches(
            Kind::RepeatInt,
            "60 60 60 60 60 60",
            "60 60 55 60 60 60"
        ));
    }

    #[test]
    fn flagyn_readback_flexible() {
        assert!(readback_matches(Kind::FlagYN, "Y", "Y"));
        assert!(readback_matches(Kind::FlagYN, "N", "0"));
        assert!(!readback_matches(Kind::FlagYN, "Y", "N"));
    }

    #[test]
    fn io_sched_readback_requires_active_bracket() {
        // only the BRACKETED token is the active scheduler; a bare token in
        // the offered list must NOT count (real bug: "deadline" matched while
        // cfq was active and the elevator never switched).
        assert!(readback_matches(
            Kind::IoSched,
            "cfq",
            "noop deadline [cfq]"
        ));
        assert!(readback_matches(
            Kind::IoSched,
            "deadline",
            "noop [deadline] cfq"
        ));
        assert!(!readback_matches(
            Kind::IoSched,
            "deadline",
            "noop deadline [cfq]"
        ));
        assert!(!readback_matches(Kind::IoSched, "cfq", "noop [deadline]"));
        assert!(!readback_matches(Kind::IoSched, "noop", "deadline [cfq]"));
        // bare (unbracketed) read-backs still compare directly
        assert!(readback_matches(Kind::IoSched, "cfq", "cfq"));
    }

    #[test]
    fn snapshot_normalize_makes_values_node_writable() {
        assert_eq!(
            normalize_snapshot(Kind::IoSched, "noop deadline [cfq]"),
            "cfq"
        );
        assert_eq!(
            normalize_snapshot(Kind::Ints, "524288\t1048576\t5505024"),
            "524288 1048576 5505024"
        );
        assert_eq!(normalize_snapshot(Kind::Mask, "0-5"), "0-5");
        assert_eq!(normalize_snapshot(Kind::FlagYN, "n"), "N");
        assert_eq!(normalize_snapshot(Kind::Int, " 65 "), "65");
        // uniform RepeatInt arrays collapse to the broadcast single value
        assert_eq!(
            normalize_snapshot(Kind::RepeatInt, "60 60 60 60 60 60"),
            "60"
        );
        assert_eq!(
            normalize_snapshot(Kind::RepeatInt, "60 40 60 60 60 60"),
            "60 40 60 60 60 60"
        );
    }

    #[test]
    fn freq_max_accepts_stricter_external_cap() {
        // thermal cooling holds scaling_max below the requested cap -> thermal
        // wins (harmony rule), reported as in-sync, not as drift.
        assert!(readback_matches(Kind::FreqMax, "1555200", "1209600"));
        assert!(readback_matches(Kind::FreqMax, "1555200", "1555200"));
        assert!(!readback_matches(Kind::FreqMax, "1555200", "1804800"));
        // plain Freq (e.g. scaling_min_freq) stays exact
        assert!(!readback_matches(Kind::Freq, "1555200", "1209600"));
    }

    #[test]
    fn freq_min_accepts_external_qos_floor() {
        // perf HAL msm_performance cpu_min_freq QoS holds the effective floor
        // ABOVE the requested value -> framework wins (harmony rule), in-sync,
        // not drift. Only live < want (floor lost) is drift.
        // Live case 2026-10-07: wrote 576000, read back 1248000 under QoS.
        assert!(readback_matches(Kind::FreqMin, "576000", "1248000"));
        assert!(readback_matches(Kind::FreqMin, "576000", "576000"));
        assert!(!readback_matches(Kind::FreqMin, "1094400", "768000"));
        // and a QoS-held floor must not leak into exact-match kinds
        assert!(!readback_matches(Kind::Freq, "576000", "1248000"));
    }
}
