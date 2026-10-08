//! Per-app Do Not Disturb (official API bridge).
//!
//! `zen_mode` is framework-owned (etag-tracked by NotificationManagerService)
//! and `cmd notification set_dnd` does not exist on Android 10, so the daemon
//! never writes it: it decides *when* and emits a `dnd` event. The app
//! service executes through `NotificationManager.setInterruptionFilter()`
//! (after the user grants "Do Not Disturb access") and restores the previous
//! filter on release — the framework updates `zen_mode` + etag itself.

use super::sync::fg_app_profile;
use super::{Bridge, State, SyncCtx};
use crate::daemon::proto::Event;

/// Pure gate: Dynamic ON + visible & unlocked + DND access granted + a valid
/// per-app mode.
pub fn dnd_want(ctx: &SyncCtx) -> Option<String> {
    if !(ctx.dynamic && ctx.screen_on && !ctx.locked && ctx.dnd_granted) {
        return None;
    }
    let ap = fg_app_profile(ctx)?;
    ap.dnd_mode().map(str::to_string)
}

impl Bridge {
    /// Emits the desired DND mode when it changes; the app applies/restores.
    pub(super) fn sync_dnd(&self, st: &mut State, ctx: &SyncCtx) -> bool {
        let want = dnd_want(ctx);
        if want == st.dnd_sent {
            return false;
        }
        self.publisher.emit(&Event::Dnd { mode: want.clone() });
        match (&want, st.dnd_sent.is_some()) {
            (Some(m), _) => self.log_event(format!(
                "DND {m} ({})",
                ctx.last_real.as_deref().unwrap_or("?")
            )),
            (None, true) => self.log_event("DND released".into()),
            (None, false) => {}
        }
        st.dnd_sent = want;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::config::AppProfile;

    fn ctx(apps: &[(&str, AppProfile)], fg: Option<&str>, granted: bool) -> SyncCtx {
        SyncCtx {
            last_real: fg.map(str::to_string),
            screen_on: true,
            locked: false,
            dynamic: true,
            sync_perf: true,
            sync_saver: true,
            game_checker: true,
            app_map: Default::default(),
            app_profiles: apps
                .iter()
                .map(|(p, a)| (p.to_string(), a.clone()))
                .collect(),
            bypass_floor_pct: 30,
            dnd_granted: granted,
            battery_pct: None,
            charging: None,
            charge_limit: false,
            charge_limit_pct: 80,
        }
    }

    fn dnd_app() -> AppProfile {
        AppProfile {
            dnd: Some("priority".into()),
            ..Default::default()
        }
    }

    #[test]
    fn gate_requires_access_foreground_and_valid_mode() {
        let c = ctx(&[("com.g", dnd_app())], Some("com.g"), true);
        assert_eq!(dnd_want(&c), Some("priority".into()));
        // access not granted -> nothing
        assert_eq!(
            dnd_want(&ctx(&[("com.g", dnd_app())], Some("com.g"), false)),
            None
        );
        // other app / no override / invalid mode
        assert_eq!(
            dnd_want(&ctx(&[("com.g", dnd_app())], Some("com.x"), true)),
            None
        );
        assert_eq!(
            dnd_want(&ctx(
                &[("com.g", AppProfile::default())],
                Some("com.g"),
                true
            )),
            None
        );
        let bad = ctx(
            &[(
                "com.g",
                AppProfile {
                    dnd: Some("silent".into()),
                    ..Default::default()
                },
            )],
            Some("com.g"),
            true,
        );
        assert_eq!(dnd_want(&bad), None);
        // dynamic off / screen off / locked
        let mut c2 = c.clone();
        c2.dynamic = false;
        assert_eq!(dnd_want(&c2), None);
        let mut c3 = c.clone();
        c3.screen_on = false;
        assert_eq!(dnd_want(&c3), None);
        let mut c4 = c.clone();
        c4.locked = true;
        assert_eq!(dnd_want(&c4), None);
    }
}
