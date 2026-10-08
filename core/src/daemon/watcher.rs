//! Foreground + multi-window watchers: the daemon spawns `logcat` directly.
//!
//! Precision choices (vs the Kotlin implementation this replaces):
//! - `-v epoch` timestamps: freshness is epoch arithmetic — no timezone,
//!   no two-digit-year inference, no 1970-style parse bugs.
//! - capture time travels with the event, so a slow pipe cannot reorder
//!   decisions after the fact.
//!
//! The daemon runs as root already, so plain `/system/bin/logcat` works —
//! no per-stream `su` wrapper.
//!
//! Pure parsers live at the bottom and are fixture-tested; the spawn/read
//! plumbing is thin on purpose.

use std::io::BufRead;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::Msg;

const LOGCAT: &str = "/system/bin/logcat";
/// Events older than this are history, not context (peek only).
pub const MAX_PEEK_AGE_S: f64 = 60.0;

/// logcat path; overridable for host tests (fake-logcat fixtures).
fn logcat_bin() -> String {
    std::env::var("MIFINETUNE_LOGCAT_BIN").unwrap_or_else(|_| LOGCAT.to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatcherKind {
    Fg,
    Mw,
    Jank,
}

impl WatcherKind {
    pub fn name(self) -> &'static str {
        match self {
            WatcherKind::Fg => "foreground",
            WatcherKind::Mw => "multi-window",
            WatcherKind::Jank => "jank",
        }
    }
}

/// A live logcat stream with its reader thread.
pub struct Watcher {
    child: Child,
    alive: Arc<AtomicBool>,
}

impl Watcher {
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Relaxed)
    }

    pub fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.alive.store(false, Ordering::Relaxed);
    }
}

/// Spawns the foreground event stream (resume events) and feeds `Msg::Fg`
/// into the daemon loop.
pub fn spawn_fg(tx: Sender<Msg>) -> Option<Watcher> {
    spawn(
        WatcherKind::Fg,
        &[
            "-b",
            "events",
            "-v",
            "epoch",
            "-s",
            "am_resume_activity:V",
            "am_set_resumed_activity:V",
        ],
        tx,
        |line, tx| {
            if let Some((epoch, pkg)) = parse_fg_line(line) {
                if is_fresh(epoch, now_epoch()) {
                    let _ = tx.send(Msg::Fg(pkg));
                }
            }
        },
    )
}

/// Spawns the multi-window stream (`GameBoosterService` status lines).
/// The buffer replay IS the seed: last line = current state (split mode
/// persists until exited), so stale lines are not skipped here.
pub fn spawn_mw(tx: Sender<Msg>) -> Option<Watcher> {
    spawn(
        WatcherKind::Mw,
        &["-b", "main", "-v", "epoch", "-s", "GameBoosterService:V"],
        tx,
        |line, tx| {
            if let Some(other) = parse_mw_line(line) {
                let _ = tx.send(Msg::Mw {
                    active: other.is_some(),
                    other,
                });
            }
        },
    )
}

/// Spawns the jank stream (`Choreographer: Skipped N frames!`). Freshness is
/// checked so a buffer replay cannot boost at startup.
pub fn spawn_jank(tx: Sender<Msg>) -> Option<Watcher> {
    spawn(
        WatcherKind::Jank,
        &["-b", "main", "-v", "epoch", "-s", "Choreographer:V"],
        tx,
        |line, tx| {
            if let Some(frames) = parse_jank_line(line) {
                if line_epoch(line)
                    .map(|e| is_fresh(e, now_epoch()))
                    .unwrap_or(false)
                {
                    let _ = tx.send(Msg::Jank { frames });
                }
            }
        },
    )
}

fn spawn(
    kind: WatcherKind,
    args: &[&str],
    tx: Sender<Msg>,
    mut on_line: impl FnMut(&str, &Sender<Msg>) + Send + 'static,
) -> Option<Watcher> {
    let mut child = Command::new(logcat_bin())
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let stdout = child.stdout.take()?;
    let alive = Arc::new(AtomicBool::new(true));
    let alive2 = alive.clone();
    std::thread::Builder::new()
        .name(format!("watcher-{}", kind.name()))
        .spawn(move || {
            let reader = std::io::BufReader::new(stdout);
            for line in reader.lines() {
                let Ok(line) = line else { break };
                on_line(&line, &tx);
            }
            alive2.store(false, Ordering::Relaxed);
            let _ = tx.send(Msg::WatcherDown(kind));
        })
        .ok()?;
    let _ = kind;
    Some(Watcher { child, alive })
}

/// One-shot peek of the most recent fresh resume event.
/// Blocking — call from a short-lived thread. Uses the `-d` (dump) mode.
pub fn peek_fg() -> Option<String> {
    let out = Command::new(logcat_bin())
        .args([
            "-b",
            "events",
            "-v",
            "epoch",
            "-d",
            "-s",
            "am_resume_activity:V",
            "am_set_resumed_activity:V",
        ])
        .stdin(Stdio::null())
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let now = now_epoch();
    text.lines()
        .filter_map(parse_fg_line)
        .rfind(|(epoch, _)| is_fresh(*epoch, now))
        .map(|(_, pkg)| pkg)
}

pub fn now_epoch() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// Age check in epoch seconds: 0 <= age <= MAX (future stamps are stale).
pub fn is_fresh(epoch: f64, now: f64) -> bool {
    let age = now - epoch;
    (0.0..=MAX_PEEK_AGE_S).contains(&age)
}

/// First whitespace token of a `-v epoch` line -> epoch seconds.
fn line_epoch(line: &str) -> Option<f64> {
    line.split_whitespace().next()?.parse().ok()
}

/// Nth comma-separated field of `rest`, up to the first `/` (pkg/class).
fn tuple_field(rest: &str, n: usize) -> Option<String> {
    let field = rest.split(',').nth(n)?.trim();
    let pkg = field.split('/').next()?.trim();
    if pkg.is_empty() {
        None
    } else {
        Some(pkg.to_string())
    }
}

/// `... am_resume_activity: [user, token, task, pkg/class, pid]`
/// `... am_set_resumed_activity: [user, pkg/class, reason]`
pub fn parse_fg_line(line: &str) -> Option<(f64, String)> {
    let epoch = line_epoch(line)?;
    if let Some(idx) = line.find("am_resume_activity: [") {
        let rest = &line[idx + "am_resume_activity: [".len()..];
        return tuple_field(rest, 3).map(|pkg| (epoch, pkg));
    }
    if let Some(idx) = line.find("am_set_resumed_activity: [") {
        let rest = &line[idx + "am_set_resumed_activity: [".len()..];
        return tuple_field(rest, 1).map(|pkg| (epoch, pkg));
    }
    None
}

/// `mMultiWindowForegroundPackageName='<pkg>|null'` -> Some(pkg) / None (off).
/// Returns Option<Option<..>>: outer = line matched, inner = pane present.
pub fn parse_mw_line(line: &str) -> Option<Option<String>> {
    const KEY: &str = "mMultiWindowForegroundPackageName='";
    let idx = line.find(KEY)?;
    let rest = &line[idx + KEY.len()..];
    let end = rest.find('\'')?;
    let name = &rest[..end];
    if name == "null" {
        Some(None)
    } else {
        Some(Some(name.to_string()))
    }
}

/// `Choreographer: Skipped N frames!` -> Some(N).
pub fn parse_jank_line(line: &str) -> Option<u32> {
    const KEY: &str = "Choreographer: Skipped ";
    let idx = line.find(KEY)?;
    let rest = &line[idx + KEY.len()..];
    rest.split_whitespace().next()?.parse().ok()
}

/// Restart backoff for dead streams (matches the old Java-side policy).
pub const RESTART_BACKOFF: Duration = Duration::from_secs(10);

#[cfg(test)]
mod tests {
    use super::*;

    // Fixtures below are real `logcat -b events -v epoch` shapes from surya.

    #[test]
    fn parse_resume_activity_line() {
        let line = "1760022735.123  1234  1234 I am_resume_activity: [0,10001,2830,com.YoStarEN.AzurLane/.MainActivity,5678]";
        let (epoch, pkg) = parse_fg_line(line).unwrap();
        assert!((epoch - 1760022735.123).abs() < 0.001);
        assert_eq!(pkg, "com.YoStarEN.AzurLane");
    }

    #[test]
    fn parse_set_resumed_activity_line() {
        // monkey/new-task launches log ONLY this one (verified 2026-10-08)
        let line = "1760022735.123  1234  1234 I am_set_resumed_activity: [0,com.YoStarEN.AzurLane/.MainActivity,startActivity]";
        let (_, pkg) = parse_fg_line(line).unwrap();
        assert_eq!(pkg, "com.YoStarEN.AzurLane");
    }

    #[test]
    fn parse_rejects_unrelated_lines() {
        assert!(
            parse_fg_line("1760022735.123  1234  1234 I am_pause_activity: [0,123,com.x/.Y]")
                .is_none()
        );
        assert!(parse_fg_line("").is_none());
        assert!(parse_fg_line("no-epoch am_resume_activity: [0,1,2,com.x/.Y,9]").is_none());
    }

    #[test]
    fn freshness_window_is_epoch_arithmetic() {
        let now = 1_000_000.0;
        assert!(is_fresh(now - 5.0, now));
        assert!(is_fresh(now, now));
        assert!(!is_fresh(now - 61.0, now));
        assert!(!is_fresh(now + 1.0, now), "future stamps are stale");
    }

    #[test]
    fn parse_multi_window_state() {
        let on = "1760022735.123  1234  1234 D GameBoosterService: onGameStatusChange id=1 mForegroundPackageName='com.google.android.youtube' mMultiWindowForegroundPackageName='com.android.chrome'";
        assert_eq!(
            parse_mw_line(on).unwrap(),
            Some("com.android.chrome".to_string())
        );
        let off = "1760022735.123  1234  1234 D GameBoosterService: onGameStatusChange id=1 mForegroundPackageName='com.miui.home' mMultiWindowForegroundPackageName='null'";
        assert_eq!(parse_mw_line(off).unwrap(), None);
        assert!(parse_mw_line("unrelated line").is_none());
    }

    #[test]
    fn parse_choreographer_jank_line() {
        let line = "1791471427.395 12072 12072 I Choreographer: Skipped 93 frames!  The application may be doing too much work on its main thread.";
        assert_eq!(parse_jank_line(line), Some(93));
        assert_eq!(
            parse_jank_line("1.0 1 1 I Choreographer: Skipped 1 frames!"),
            Some(1)
        );
        assert!(parse_jank_line("1.0 1 1 I Choreographer: doing work").is_none());
        assert!(parse_jank_line("unrelated").is_none());
    }
}
