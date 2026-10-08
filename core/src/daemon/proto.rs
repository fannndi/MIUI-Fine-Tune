//! Daemon IPC protocol: JSON-lines over stdio.
//!
//! Commands (app -> daemon) and events (daemon -> app) are tagged enums
//! serialized one-per-line. Unknown commands are logged and ignored — a
//! protocol mismatch must never crash the daemon (precision policy).
//!
//! Logging: human-readable lines go to stderr (the app relays them to
//! logcat under the `MiFineTune` tag); stdout carries ONLY protocol JSON.

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::sync::{Arc, Mutex};

/// Protocol version — bumped on breaking changes; the app checks `hello`.
pub const VERSION: u32 = 1;

// --- commands (app -> daemon) ----------------------------------------------

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Command {
    /// Handshake after spawn; daemon answers with a `state` event.
    Hello,
    /// config.json changed on disk — reload + re-evaluate.
    ConfigChanged,
    /// Screen state (broadcast or reconcile tick).
    Screen { on: bool, locked: bool },
    /// Keyguard dismissed (also implies screen on).
    UserPresent,
    /// Raw foreground event forwarded by the app watcher.
    Fg { pkg: String },
    /// Wake/unlock seed: latest peeked package (None = peek found nothing).
    Seed { pkg: Option<String> },
    /// Multi-window state forwarded by the app watcher.
    Mw { active: bool, other: Option<String> },
    /// MIUI Ultra battery saver broadcast.
    Ultra { on: bool },
    /// Manual card tap: apply now and treat as the universal base.
    SetBase { profile: String },
    /// Service going off: drop pending applies, restore stock, keep running.
    Restore,
    /// Graceful shutdown (service destroyed on purpose).
    Shutdown,
    /// Liveness check -> `pong`.
    Ping,
}

// --- events (daemon -> app) -------------------------------------------------

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    /// First event after startup.
    Hello { version: u32, pid: u32 },
    /// What the arbiter decided and why (diagnostics + host tests).
    Decision {
        trigger: String,
        action: String, // "none" | "apply" | "retire"
        #[serde(skip_serializing_if = "Option::is_none")]
        profile: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    /// Profile applied (or confirmed already active). `reason` is raw:
    /// "base" | "screen off" | "app" | "multi-window" | "MIUI saver";
    /// the app resolves labels for "app" via `src_pkg`.
    Applied {
        profile: String,
        reason: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        src_pkg: Option<String>,
        ok: bool,
        wrote: usize,
        failed: usize,
        ms: u64,
        settle_ms: u64,
    },
    /// Full runtime snapshot (emitted when anything visible changes).
    State { state: Snapshot },
    /// Bridge timeline entry (MIUI mode writes).
    Bridge { msg: String },
    /// MIUI Game Booster conflict for a mapped game (warn once per session).
    GameModeConflict { pkg: String },
    /// Ultra saver retire finished: the app must set enabled=false + stop.
    Retired { ok: bool, failed: usize },
    /// Restore finished (service-off path).
    Restored { ok: bool, failed: usize },
    /// Ping reply.
    Pong,
    /// Non-fatal error for the log (daemon keeps running).
    Error { msg: String },
    /// Graceful exit.
    Bye,
}

/// Runtime snapshot mirrored into the app's `DynamicProfileState`.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Snapshot {
    pub screen_on: bool,
    pub locked: bool,
    pub multi_window: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub second_window: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub foreground: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub src_pkg: Option<String>,
}

// --- emission ---------------------------------------------------------------

/// Serialized JSON-line writer shared by all threads. One `emit` = one
/// flushed line; ordering between threads is whichever locks first.
pub struct Publisher {
    out: Mutex<std::io::Stdout>,
}

impl Publisher {
    pub fn new() -> Arc<Self> {
        Arc::new(Publisher { out: Mutex::new(std::io::stdout()) })
    }

    pub fn emit(&self, ev: &Event) {
        let line = match serde_json::to_string(ev) {
            Ok(l) => l,
            Err(e) => {
                self.log(&format!("event serialize failed: {e}"));
                return;
            }
        };
        let mut out = self.out.lock().unwrap_or_else(|p| p.into_inner());
        let _ = writeln!(out, "{line}");
        let _ = out.flush();
    }

    /// Human log line on stderr (relayed to logcat by the app).
    pub fn log(&self, msg: &str) {
        eprintln!("{msg}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_parse_from_tagged_json() {
        let c: Command = serde_json::from_str(r#"{"cmd":"screen","on":false,"locked":true}"#).unwrap();
        assert_eq!(c, Command::Screen { on: false, locked: true });

        let c: Command = serde_json::from_str(r#"{"cmd":"seed","pkg":null}"#).unwrap();
        assert_eq!(c, Command::Seed { pkg: None });

        let c: Command = serde_json::from_str(r#"{"cmd":"set_base","profile":"game"}"#).unwrap();
        assert_eq!(c, Command::SetBase { profile: "game".into() });
    }

    #[test]
    fn unknown_command_is_rejected_not_panicked() {
        let r = serde_json::from_str::<Command>(r#"{"cmd":"nope"}"#);
        assert!(r.is_err());
    }

    #[test]
    fn events_serialize_with_tag_and_skip_none() {
        let ev = Event::Decision {
            trigger: "event".into(),
            action: "apply".into(),
            profile: Some("game".into()),
            reason: None,
        };
        let s = serde_json::to_string(&ev).unwrap();
        assert!(s.contains(r#""event":"decision""#));
        assert!(s.contains(r#""trigger":"event""#));
        assert!(!s.contains("reason")); // None is skipped
    }

    #[test]
    fn snapshot_default_is_all_off() {
        let s = Snapshot::default();
        assert!(!s.screen_on);
        assert!(s.active.is_none());
    }
}
