//! Pure decision table — the port of the Kotlin `ModeArbiter`.
//!
//! Model: the last manually chosen profile is the universal BASE; apps with
//! a per-app mapping override it while they are in front; screen-off always
//! applies the sleep profile. Pure data in, one decision out — no IO.

/// Screen-off profile.
pub const SLEEP_PROFILE: &str = "sleep";
/// MIUI/Android battery saver forces the frugal base.
pub const SAVER_PROFILE: &str = "powersave";
/// Dual visible apps (split/floating) are always tuned Balanced.
pub const MULTI_WINDOW_PROFILE: &str = "balance";

#[derive(Debug, Clone, PartialEq)]
pub struct ArbiterInput {
    pub service_enabled: bool,
    pub screen_on: bool,
    pub keyguard_locked: bool,
    pub foreground_pkg: Option<String>,
    pub app_map: std::collections::BTreeMap<String, String>,
    pub base_profile: String,
    pub sleep_profile: String,
    /// MIUI battery saver / Android battery saver — forces the frugal base.
    pub saver_on: bool,
    /// MIUI Ultra battery saver — framework owns the device; we retire.
    pub ultra_saver: bool,
    /// Split screen / floating window active — dual-app concurrency.
    pub multi_window: bool,
    /// Dynamic Profile: ON = mapped app overrides the base; OFF = base wins.
    pub dynamic_profile: bool,
}

impl Default for ArbiterInput {
    fn default() -> Self {
        ArbiterInput {
            service_enabled: true,
            screen_on: true,
            keyguard_locked: false,
            foreground_pkg: None,
            app_map: Default::default(),
            base_profile: "balance".into(),
            sleep_profile: SLEEP_PROFILE.into(),
            saver_on: false,
            ultra_saver: false,
            multi_window: false,
            dynamic_profile: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    /// Leave whatever is active in place.
    None,
    /// Apply `profile`; `reason` is a stable machine label.
    Apply { profile: String, reason: String },
    /// Framework owns tuning (Ultra battery saver) — restore + stand down.
    Retire,
}

/// Packages that must not trigger a switch: system chrome and dialogs
/// appear "in front" of the real foreground app. IMEs match by substring.
/// The launcher is NOT transient — leaving an app to the launcher is the
/// signal to go back to the base.
const TRANSIENT_PACKAGES: &[&str] = &[
    "com.android.systemui",
    "com.mifinetune",
    "com.android.permissioncontroller",
    "com.google.android.permissioncontroller",
    "com.lbe.security.miui", // MIUI permission dialogs
    "android",
    "com.baidu.input_mi", // MIUI-common IME without "inputmethod" in the name
    // the bridge's own performance-follow surface (hidden sheet) — its
    // resume event must not read as "user is in Settings"
    "com.android.settings",
];

pub fn is_transient(pkg: Option<&str>) -> bool {
    match pkg {
        None => true,
        Some(p) => {
            TRANSIENT_PACKAGES.contains(&p) || p.to_ascii_lowercase().contains("inputmethod")
        }
    }
}

pub fn decide(i: &ArbiterInput) -> Decision {
    if !i.service_enabled {
        return Decision::None;
    }

    // MIUI Ultra battery saver owns the whole device (own CPU/GPU/network
    // regime, whitelisted apps only). Not our world — retire with a restore.
    if i.ultra_saver {
        return Decision::Retire;
    }

    if !i.screen_on {
        return Decision::Apply { profile: i.sleep_profile.clone(), reason: "screen off".into() };
    }

    if i.keyguard_locked {
        return Decision::None;
    }

    // dual-app concurrency overrides mapping AND saver: two visible apps
    // need the middle ground (user verdict 2026-10-08)
    if i.multi_window {
        return Decision::Apply { profile: MULTI_WINDOW_PROFILE.into(), reason: "multi-window".into() };
    }

    let pkg = i.foreground_pkg.as_deref();
    if is_transient(pkg) {
        return Decision::None;
    }

    // Dynamic Profile OFF: the universal base wins no matter what the app
    // map says — mapping is only a hint for the dynamic mode.
    let mapped = if i.dynamic_profile {
        pkg.and_then(|p| i.app_map.get(p)).cloned()
    } else {
        None
    };
    // MIUI battery saver: the unmapped universe is forced to the frugal
    // base — mapped apps still win (the user may game under saver).
    let effective_base = if i.saver_on { SAVER_PROFILE } else { i.base_profile.as_str() };
    match mapped {
        Some(profile) => Decision::Apply { profile, reason: "app".into() },
        None => Decision::Apply { profile: effective_base.to_string(), reason: "base".into() },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::test_util::map;

    fn input(
        enabled: bool,
        screen_on: bool,
        locked: bool,
        fg: Option<&str>,
        app_map: &[(&str, &str)],
        base: &str,
        saver: bool,
        ultra: bool,
        mw: bool,
        dynamic: bool,
    ) -> ArbiterInput {
        ArbiterInput {
            service_enabled: enabled,
            screen_on,
            keyguard_locked: locked,
            foreground_pkg: fg.map(str::to_string),
            app_map: map(app_map),
            base_profile: base.into(),
            sleep_profile: SLEEP_PROFILE.into(),
            saver_on: saver,
            ultra_saver: ultra,
            multi_window: mw,
            dynamic_profile: dynamic,
        }
    }

    fn d(i: &ArbiterInput) -> Decision {
        decide(i)
    }

    fn applied(profile: &str, reason: &str) -> Decision {
        Decision::Apply { profile: profile.into(), reason: reason.into() }
    }

    // --- base table (ported 1:1 from ModeArbiterTest) ----------------------

    #[test]
    fn service_off_is_none() {
        assert_eq!(d(&input(false, true, false, Some("com.example.app"), &[], "balance", false, false, false, true)), Decision::None);
    }

    #[test]
    fn screen_off_applies_sleep() {
        assert_eq!(
            d(&input(true, false, false, Some("com.example.app"), &[], "balance", false, false, false, true)),
            applied("sleep", "screen off")
        );
    }

    #[test]
    fn keyguard_locked_is_none() {
        assert_eq!(d(&input(true, true, true, Some("com.example.app"), &[], "balance", false, false, false, true)), Decision::None);
    }

    #[test]
    fn mapped_app_applies_mapped_profile() {
        let i = input(true, true, false, Some("com.YoStarEN.AzurLane"), &[("com.YoStarEN.AzurLane", "game")], "balance", false, false, false, true);
        assert_eq!(d(&i), applied("game", "app"));
    }

    #[test]
    fn unmapped_app_applies_base() {
        let i = input(true, true, false, Some("com.whatsapp"), &[], "powersave", false, false, false, true);
        assert_eq!(d(&i), applied("powersave", "base"));
    }

    #[test]
    fn launcher_reverts_to_base() {
        let i = input(true, true, false, Some("com.miui.home"), &[], "powersave", false, false, false, true);
        assert_eq!(d(&i), applied("powersave", "base"));
    }

    #[test]
    fn system_chrome_is_transient() {
        for pkg in ["com.android.systemui", "com.mifinetune", "com.lbe.security.miui", "com.google.android.inputmethod.latin"] {
            let i = input(true, true, false, Some(pkg), &[], "balance", false, false, false, true);
            assert_eq!(d(&i), Decision::None, "{pkg}");
        }
    }

    #[test]
    fn null_foreground_is_none() {
        assert_eq!(d(&input(true, true, false, None, &[], "balance", false, false, false, true)), Decision::None);
    }

    #[test]
    fn transient_predicate_matches_ime_substring() {
        assert!(is_transient(Some("com.sohu.inputmethod.sogou.xiaomi")));
        assert!(is_transient(Some("com.baidu.input_mi")));
        assert!(is_transient(Some("com.iflytek.inputmethod.miui")));
        assert!(is_transient(Some("com.android.settings"))); // bridge sheet
        assert!(is_transient(None));
        assert!(!is_transient(Some("com.miui.home")));
    }

    // --- MIUI bridge rules ---------------------------------------------------

    #[test]
    fn saver_on_forces_powersave_base_but_mapping_still_wins() {
        let i = input(true, true, false, Some("com.example.app"), &[], "game", true, false, false, true);
        assert_eq!(d(&i), applied("powersave", "base"));

        let i = input(true, true, false, Some("com.YoStarEN.AzurLane"), &[("com.YoStarEN.AzurLane", "game")], "balance", true, false, false, true);
        assert_eq!(d(&i), applied("game", "app"));
    }

    #[test]
    fn ultra_saver_retires_above_everything() {
        let i = input(true, true, false, Some("com.example.app"), &[], "balance", false, true, false, true);
        assert_eq!(d(&i), Decision::Retire);
        let i = input(true, false, false, Some("com.example.app"), &[], "balance", false, true, false, true);
        assert_eq!(d(&i), Decision::Retire, "retire wins even over screen-off");
    }

    // --- multi-window rule -----------------------------------------------------

    #[test]
    fn multi_window_forces_balance_over_mapping() {
        let i = input(true, true, false, Some("com.YoStarEN.AzurLane"), &[("com.YoStarEN.AzurLane", "game")], "balance", false, false, true, true);
        assert_eq!(d(&i), applied("balance", "multi-window"));
    }

    #[test]
    fn multi_window_forces_balance_over_saver_base() {
        let i = input(true, true, false, Some("com.example.app"), &[], "balance", true, false, true, true);
        assert_eq!(d(&i), applied("balance", "multi-window"));
    }

    #[test]
    fn multi_window_does_not_beat_screen_off() {
        let i = input(true, false, false, Some("com.example.app"), &[], "balance", false, false, true, true);
        assert_eq!(d(&i), applied("sleep", "screen off"));
    }

    // --- dynamic profile rule ---------------------------------------------------

    #[test]
    fn dynamic_off_mapped_app_falls_back_to_base() {
        let i = input(true, true, false, Some("com.YoStarEN.AzurLane"), &[("com.YoStarEN.AzurLane", "game")], "balance", false, false, false, false);
        assert_eq!(d(&i), applied("balance", "base"));
    }

    #[test]
    fn dynamic_off_still_forces_powersave_under_saver() {
        let i = input(true, true, false, Some("com.example.app"), &[], "balance", true, false, false, false);
        assert_eq!(d(&i), applied("powersave", "base"));
    }

    #[test]
    fn dynamic_off_still_applies_sleep() {
        let i = input(true, false, false, Some("com.example.app"), &[], "balance", false, false, false, false);
        assert_eq!(d(&i), applied("sleep", "screen off"));
    }

    #[test]
    fn dynamic_off_still_forces_balance_on_multi_window() {
        let i = input(true, true, false, Some("com.example.app"), &[], "balance", false, false, true, false);
        assert_eq!(d(&i), applied("balance", "multi-window"));
    }

    #[test]
    fn dynamic_off_keeps_transient_noop() {
        let i = input(true, true, false, Some("com.android.systemui"), &[], "balance", false, false, false, false);
        assert_eq!(d(&i), Decision::None);
    }

    #[test]
    fn dynamic_on_mapped_app_applies_mapped_profile() {
        let i = input(true, true, false, Some("com.YoStarEN.AzurLane"), &[("com.YoStarEN.AzurLane", "game")], "balance", false, false, false, true);
        assert_eq!(d(&i), applied("game", "app"));
    }
}
