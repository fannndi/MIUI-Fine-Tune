//! Hold/restore state machine + persisted restore points.
//!
//! When the bridge drives a MIUI mode for a mapped app, it must never stomp
//! the user's own choice: the first hold captures the live value as the
//! restore point, the release writes it back. While a hold is active the
//! arbiter must see the USER's value, not our transient write (attribution).

use serde::{Deserialize, Serialize};

/// MIUI's own performance switch values (Settings.System mirror).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PowerMode {
    #[serde(rename = "middle")]
    Balanced,
    #[serde(rename = "high")]
    Performance,
}

impl PowerMode {
    pub fn of(raw: &str) -> PowerMode {
        if raw == "high" {
            PowerMode::Performance
        } else {
            PowerMode::Balanced
        }
    }
    pub fn key(self) -> &'static str {
        match self {
            PowerMode::Balanced => "middle",
            PowerMode::Performance => "high",
        }
    }
}
#[derive(Debug)]
pub enum PerfAction {
    None,
    Write,
    Keep,
    Restore,
}

#[derive(Debug)]
pub enum SaverAction {
    None,
    TurnOn,
    Keep,
    Restore,
}

/// Charge-guard action (`battery_charging_enabled` node).
#[derive(Debug)]
pub enum ChargeAction {
    None,
    Keep,
    /// Stop charging (write 0).
    Pause,
    /// Release: write the captured value back (None = assume "1").
    Resume(Option<String>),
}

/// Per-app bypass action (`input_suspend` node).
#[derive(Debug)]
pub enum BypassAction {
    None,
    Keep,
    /// Suspend input (write 1): the device runs on battery.
    Engage,
    /// Release: write the captured value back (None = assume "0").
    Release(Option<String>),
}

/// Hold/restore state machine (pure, unit tested).
#[derive(Debug, Clone, PartialEq)]
pub struct Holds {
    pub perf_held: bool,
    pub perf_saved: PowerMode,
    pub saver_held: bool,
    pub saver_saved: bool,
    pub charge_held: bool,
    /// The node value before the pause (None = assume "1").
    pub charge_saved: Option<String>,
    pub bypass_held: bool,
    /// The node value before the bypass (None = assume "0").
    pub bypass_saved: Option<String>,
}

impl Default for Holds {
    fn default() -> Self {
        Holds {
            perf_held: false,
            perf_saved: PowerMode::Balanced,
            saver_held: false,
            saver_saved: false,
            charge_held: false,
            charge_saved: None,
            bypass_held: false,
            bypass_saved: None,
        }
    }
}

impl Holds {
    /// `live` = current switch state; `want` = bridge wants Performance.
    pub fn request_perf(&self, live: PowerMode, want: bool) -> (Holds, PerfAction) {
        match (want, self.perf_held) {
            (true, false) => (
                Holds {
                    perf_held: true,
                    perf_saved: live,
                    ..self.clone()
                },
                if live == PowerMode::Performance {
                    PerfAction::Keep
                } else {
                    PerfAction::Write
                },
            ),
            (true, true) => (self.clone(), PerfAction::Keep),
            (false, true) => (
                Holds {
                    perf_held: false,
                    ..self.clone()
                },
                PerfAction::Restore,
            ),
            (false, false) => (self.clone(), PerfAction::None),
        }
    }

    /// `live` = current battery-saver state; `want` = bridge wants ON.
    pub fn request_saver(&self, live: bool, want: bool) -> (Holds, SaverAction) {
        match (want, self.saver_held) {
            (true, false) => (
                Holds {
                    saver_held: true,
                    saver_saved: live,
                    ..self.clone()
                },
                if live {
                    SaverAction::Keep
                } else {
                    SaverAction::TurnOn
                },
            ),
            (true, true) => (self.clone(), SaverAction::Keep),
            (false, true) => (
                Holds {
                    saver_held: false,
                    ..self.clone()
                },
                SaverAction::Restore,
            ),
            (false, false) => (self.clone(), SaverAction::None),
        }
    }

    /// The saver value the arbiter should treat as USER intent: while we
    /// hold the flag for a mapped app, the captured value stands in.
    pub fn user_saver(&self, live: bool) -> bool {
        if self.saver_held {
            self.saver_saved
        } else {
            live
        }
    }

    /// `live` = current `battery_charging_enabled`; `want_pause` = the guard
    /// wants charging stopped. Capture on the first pause, write back on
    /// release (a missing capture assumes "1" — stock).
    pub fn request_charge(&self, live: Option<String>, want_pause: bool) -> (Holds, ChargeAction) {
        match (want_pause, self.charge_held) {
            (true, false) => (
                Holds {
                    charge_held: true,
                    charge_saved: live.clone(),
                    ..self.clone()
                },
                if live.as_deref() == Some("0") {
                    ChargeAction::Keep
                } else {
                    ChargeAction::Pause
                },
            ),
            (true, true) => (self.clone(), ChargeAction::Keep),
            (false, true) => (
                Holds {
                    charge_held: false,
                    charge_saved: None,
                    ..self.clone()
                },
                ChargeAction::Resume(self.charge_saved.clone()),
            ),
            (false, false) => (self.clone(), ChargeAction::None),
        }
    }

    /// `live` = current `input_suspend`; `want` = the per-app bypass wants
    /// the input suspended. Capture on the first engage, write back on
    /// release (a missing capture assumes "0" — stock).
    pub fn request_bypass(&self, live: Option<String>, want: bool) -> (Holds, BypassAction) {
        match (want, self.bypass_held) {
            (true, false) => (
                Holds {
                    bypass_held: true,
                    bypass_saved: live.clone(),
                    ..self.clone()
                },
                if live.as_deref() == Some("1") {
                    BypassAction::Keep
                } else {
                    BypassAction::Engage
                },
            ),
            (true, true) => (self.clone(), BypassAction::Keep),
            (false, true) => (
                Holds {
                    bypass_held: false,
                    bypass_saved: None,
                    ..self.clone()
                },
                BypassAction::Release(self.bypass_saved.clone()),
            ),
            (false, false) => (self.clone(), BypassAction::None),
        }
    }
}

/// Persisted holds (crash-safe restore points), `holds.json` in the state dir.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HoldsFile {
    pub perf_held: bool,
    pub perf_saved: String,
    pub saver_held: bool,
    pub saver_saved: bool,
    #[serde(default)]
    pub charge_held: bool,
    #[serde(default)]
    pub charge_saved: Option<String>,
    #[serde(default)]
    pub bypass_held: bool,
    #[serde(default)]
    pub bypass_saved: Option<String>,
}

/// Read-only hold view for diagnostics (`diag` command); never mutates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct HoldsInfo {
    pub perf_held: bool,
    pub perf_saved: String,
    pub saver_held: bool,
    pub saver_saved: bool,
    pub charge_held: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub charge_saved: Option<String>,
    pub bypass_held: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bypass_saved: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dnd_mode: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- state machine (ported 1:1 from MiBridgeStateTest) -----------------

    #[test]
    fn perf_hold_captures_user_value_then_restores_it() {
        let s = Holds::default();
        // user was on Balanced, game enters -> WRITE
        let (n, a) = s.request_perf(PowerMode::Balanced, true);
        assert!(matches!(a, PerfAction::Write));
        assert!(n.perf_held);
        assert_eq!(n.perf_saved, PowerMode::Balanced);

        // while held, repeated requests keep (no flash spam)
        let (n2, a2) = n.request_perf(PowerMode::Performance, true);
        assert!(matches!(a2, PerfAction::Keep));
        assert_eq!(n2.perf_saved, PowerMode::Balanced);

        // game leaves -> RESTORE to the captured user value
        let (n3, a3) = n2.request_perf(PowerMode::Performance, false);
        assert!(matches!(a3, PerfAction::Restore));
        assert!(!n3.perf_held);
        assert_eq!(n3.perf_saved, PowerMode::Balanced);
    }

    #[test]
    fn perf_hold_when_user_already_on_performance_keeps_without_write() {
        let s = Holds::default();
        let (n, a) = s.request_perf(PowerMode::Performance, true);
        assert!(matches!(a, PerfAction::Keep));
        assert!(n.perf_held);
        assert_eq!(n.perf_saved, PowerMode::Performance);
    }

    #[test]
    fn saver_hold_turns_on_then_restores() {
        let s = Holds::default();
        let (n, a) = s.request_saver(false, true);
        assert!(matches!(a, SaverAction::TurnOn));
        assert!(n.saver_held);
        assert!(!n.saver_saved);

        let (n2, a2) = n.request_saver(true, false);
        assert!(matches!(a2, SaverAction::Restore));
        assert!(!n2.saver_held);
    }

    #[test]
    fn user_saver_attribution_our_hold_is_invisible_to_arbiter() {
        let s = Holds::default();
        let (n, _) = s.request_saver(false, true);
        // while held, the arbiter must NOT see our write as user intent
        assert!(!n.user_saver(true));
        // after release it sees the live flag again
        let (n2, _) = n.request_saver(true, false);
        assert!(n2.user_saver(true));
    }

    #[test]
    fn user_saver_own_choice_passes_through() {
        let s = Holds::default();
        assert!(s.user_saver(true));
        assert!(!s.user_saver(false));
    }

    #[test]
    fn recovered_hold_restores_captured_value_not_live() {
        // daemon died while holding (user's saver was off, ours is on)
        let s = Holds {
            saver_held: true,
            saver_saved: false,
            ..Default::default()
        };
        let (n, a) = s.request_saver(true, false);
        assert!(matches!(a, SaverAction::Restore));
        assert!(!n.saver_saved);
    }

    #[test]
    fn charge_hold_pauses_and_resumes() {
        let s = Holds::default();
        let (n, a) = s.request_charge(Some("1".into()), true);
        assert!(matches!(a, ChargeAction::Pause));
        assert!(n.charge_held);
        assert_eq!(n.charge_saved.as_deref(), Some("1"));

        // still wanted while held -> keep (no repeated writes)
        let (n2, a2) = n.request_charge(Some("0".into()), true);
        assert!(matches!(a2, ChargeAction::Keep));
        assert_eq!(
            n2.charge_saved.as_deref(),
            Some("1"),
            "capture must not move"
        );

        // release -> resume to the captured 1
        let (n3, a3) = n2.request_charge(Some("0".into()), false);
        assert!(matches!(a3, ChargeAction::Resume(Some(ref v)) if v == "1"));
        assert!(!n3.charge_held);
        assert!(n3.charge_saved.is_none());
    }

    #[test]
    fn charge_hold_assumes_stock_when_capture_missing() {
        let s = Holds::default();
        let (n, a) = s.request_charge(None, true);
        assert!(matches!(a, ChargeAction::Pause));
        assert!(n.charge_saved.is_none());
        let (_, a2) = n.request_charge(Some("0".into()), false);
        assert!(matches!(a2, ChargeAction::Resume(None)));
    }

    #[test]
    fn bypass_hold_engages_and_releases() {
        let s = Holds::default();
        let (n, a) = s.request_bypass(Some("0".into()), true);
        assert!(matches!(a, BypassAction::Engage));
        assert!(n.bypass_held);
        assert_eq!(n.bypass_saved.as_deref(), Some("0"));

        // still wanted -> keep (no repeated writes, capture frozen)
        let (n2, a2) = n.request_bypass(Some("1".into()), true);
        assert!(matches!(a2, BypassAction::Keep));
        assert_eq!(n2.bypass_saved.as_deref(), Some("0"));

        // release -> restore the captured 0
        let (n3, a3) = n2.request_bypass(Some("1".into()), false);
        assert!(matches!(a3, BypassAction::Release(Some(ref v)) if v == "0"));
        assert!(!n3.bypass_held);
        assert!(n3.bypass_saved.is_none());
    }

    #[test]
    fn bypass_hold_assumes_stock_when_capture_missing() {
        let s = Holds::default();
        let (n, a) = s.request_bypass(None, true);
        assert!(matches!(a, BypassAction::Engage));
        assert!(n.bypass_saved.is_none());
        let (_, a2) = n.request_bypass(Some("1".into()), false);
        assert!(matches!(a2, BypassAction::Release(None)));
    }

    #[test]
    fn power_mode_roundtrip() {
        assert_eq!(PowerMode::of("high"), PowerMode::Performance);
        assert_eq!(PowerMode::of("middle"), PowerMode::Balanced);
        assert_eq!(PowerMode::of("anything-else"), PowerMode::Balanced);
        assert_eq!(PowerMode::Performance.key(), "high");
    }

    // --- pure gate rules (dynamic/profile/screen combinations) -------------
}
