//! Apply worker: the coalescing loop that decides *when* a profile is
//! applied. Runs on its own thread; the engine (in-process) executes.
//!
//! Semantics (ported from `DynamicProfileService`):
//! - one app transition emits its event pair within ~50 ms, so a short
//!   settle window absorbs bursts — the LATEST decision in the window wins;
//! - a decision arriving while an apply runs supersedes the pending one;
//! - a failed apply (MIUI race) is retried exactly once after a short pause;
//! - `Restore` drops any pending applies and restores stock (service off).
//!
//! Timing is measured with `Instant` (monotonic): wall-clock jumps must
//! never distort settle/latency numbers.

use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

/// Coalescing window for one app transition (measured on device 2026-10-08:
/// same accuracy as 700 ms, ~300 ms faster).
pub const SETTLE_MS: u64 = 400;
/// Pause before the single retry of a failed apply.
pub const RETRY_MS: u64 = 2_000;

/// One decision queued for execution.
#[derive(Debug, Clone, PartialEq)]
pub struct Job {
    pub profile: String,
    pub reason: String,
    pub src_pkg: Option<String>,
    pub used_saver: bool,
    /// When the producer queued this job (monotonic) — settles the latency.
    pub queued: Instant,
}

/// Work items the worker accepts.
#[derive(Debug, Clone, PartialEq)]
pub enum Work {
    Apply(Job),
    /// Drop pending applies, write the stock snapshot back, release holds.
    /// `retire` marks the Ultra-saver path (app must disable + stop).
    Restore { retire: bool },
}

/// Result of a completed apply (the app turns this into UI + notification).
#[derive(Debug, Clone, PartialEq)]
pub struct AppliedEvent {
    pub profile: String,
    pub reason: String,
    pub src_pkg: Option<String>,
    pub ok: bool,
    pub wrote: usize,
    pub verified: usize,
    pub failed: usize,
    pub ms: u64,
    pub settle_ms: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RestoredEvent {
    pub retire: bool,
    pub ok: bool,
    pub wrote: usize,
    pub verified: usize,
    pub failed: usize,
}

/// Outcome of one engine apply/restore call.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub ok: bool,
    pub wrote: usize,
    pub verified: usize,
    pub failed: usize,
    pub error: Option<String>,
    /// True when the target profile was already active (no write happened).
    pub already: bool,
}

impl Outcome {
    pub fn err(msg: String) -> Self {
        Outcome { ok: false, wrote: 0, verified: 0, failed: 0, error: Some(msg), already: false }
    }
}

/// Everything the worker needs from the engine layer — injected so the
/// coalescing logic is unit-testable without a device.
pub trait EngineDriver: Send {
    /// Currently active profile id (engine state), None when stock.
    fn active(&mut self) -> Option<String>;
    /// Apply a profile (engine handles snapshot/pass-2 internally).
    fn apply(&mut self, profile_id: &str) -> Outcome;
    /// Restore the stock snapshot.
    fn restore(&mut self) -> Outcome;
    /// Release MIUI bridge holds (no-op until the bridge lands in F4).
    fn release_holds(&mut self) {}
}

/// Runs the worker loop on the calling thread until the channel closes.
/// Blocking by design — spawn a dedicated thread.
pub fn run(
    rx: Receiver<Work>,
    engine: &mut dyn EngineDriver,
    settle: Duration,
    retry: Duration,
    on_applied: &mut dyn FnMut(AppliedEvent),
    on_restored: &mut dyn FnMut(RestoredEvent),
) {
    while let Ok(first) = rx.recv() {
        match first {
            Work::Restore { retire } => {
                // drop pending applies: a restore must be final
                while let Ok(Work::Apply(_)) = rx.try_recv() {}
                engine.release_holds();
                let out = engine.restore();
                on_restored(RestoredEvent { retire, ok: out.ok, wrote: out.wrote, verified: out.verified, failed: out.failed });
            }
            Work::Apply(first_job) => {
                let mut job = first_job;
                let batch_start = job.queued;

                // settle window: latest decision wins, no extension on new arrivals
                let deadline = Instant::now() + settle;
                let mut restore_deferred: Option<Restore> = None;
                loop {
                    let now = Instant::now();
                    if now >= deadline {
                        break;
                    }
                    match rx.recv_timeout(deadline - now) {
                        Ok(Work::Apply(next)) => job = next,
                        Ok(Work::Restore { retire }) => {
                            restore_deferred = Some(Restore { retire });
                            break;
                        }
                        Err(RecvTimeoutError::Timeout) => break,
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                }

                // a restore in the window cancels this batch (never apply
                // after a restore — it would re-tune a restored device).
                if let Some(r) = restore_deferred {
                    engine.release_holds();
                    let out = engine.restore();
                    on_restored(RestoredEvent { retire: r.retire, ok: out.ok, wrote: out.wrote, verified: out.verified, failed: out.failed });
                    continue;
                }

                let settle_ms = batch_start.elapsed().as_millis() as u64;
                let started = Instant::now();
                let mut out = run_once(engine, &job);
                if !out.ok && out.error.is_none() && out.failed > 0 {
                    // transient race with MIUI writes -> one retry
                    std::thread::sleep(retry);
                    out = run_once(engine, &job);
                }
                on_applied(AppliedEvent {
                    profile: job.profile.clone(),
                    reason: reason_text(job.used_saver, &job.reason),
                    src_pkg: job.src_pkg.clone(),
                    ok: out.ok,
                    wrote: out.wrote,
                    verified: out.verified,
                    failed: out.failed,
                    ms: started.elapsed().as_millis() as u64,
                    settle_ms,
                });
            }
        }
    }
}

struct Restore {
    retire: bool,
}

fn run_once(engine: &mut dyn EngineDriver, job: &Job) -> Outcome {
    if engine.active().as_deref() == Some(job.profile.as_str()) {
        return Outcome { ok: true, wrote: 0, verified: 0, failed: 0, error: None, already: true };
    }
    engine.apply(&job.profile)
}

/// Raw reason labels the app resolves for display ("app" carries src_pkg).
fn reason_text(used_saver: bool, reason: &str) -> String {
    if used_saver && reason == "base" {
        "MIUI saver".into()
    } else {
        reason.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn ok_out(wrote: usize) -> Outcome {
        Outcome { ok: true, wrote, verified: wrote, failed: 0, error: None, already: false }
    }

    fn fail_out(errored: bool) -> Outcome {
        Outcome { ok: false, wrote: 1, verified: 0, failed: 1, error: errored.then(|| "boom".into()), already: false }
    }

    /// Scriptable engine: records calls, returns scripted outcomes.
    #[derive(Default)]
    struct FakeEngine {
        active: Option<String>,
        applies: Vec<String>,
        restores: usize,
        holds_released: usize,
        /// Outcomes returned in order; the last one repeats.
        script: Vec<Outcome>,
    }

    impl FakeEngine {
        fn with_script(script: Vec<Outcome>) -> Self {
            FakeEngine { script, ..Default::default() }
        }
        fn next_outcome(&mut self) -> Outcome {
            match self.script.len() {
                0 => ok_out(1),
                1 => self.script[0].clone(),
                _ => self.script.remove(0),
            }
        }
    }

    impl EngineDriver for FakeEngine {
        fn active(&mut self) -> Option<String> {
            self.active.clone()
        }
        fn apply(&mut self, profile_id: &str) -> Outcome {
            self.applies.push(profile_id.to_string());
            let out = self.next_outcome();
            // mirror the real engine: active only moves on success
            if out.ok {
                self.active = Some(profile_id.to_string());
            }
            out
        }
        fn restore(&mut self) -> Outcome {
            self.restores += 1;
            self.active = None;
            ok_out(2)
        }
        fn release_holds(&mut self) {
            self.holds_released += 1;
        }
    }

    fn job(profile: &str) -> Work {
        Work::Apply(Job {
            profile: profile.into(),
            reason: "app".into(),
            src_pkg: Some("com.x".into()),
            used_saver: false,
            queued: Instant::now(),
        })
    }

    #[test]
    fn burst_collapses_to_latest() {
        let (tx, rx) = mpsc::channel();
        tx.send(job("game")).unwrap();
        tx.send(job("balance")).unwrap();
        tx.send(job("powersave")).unwrap();
        drop(tx);

        let mut eng = FakeEngine::with_script(vec![ok_out(1)]);
        let mut applied = Vec::new();
        run(rx, &mut eng, Duration::from_millis(60), Duration::from_millis(5), &mut |e| applied.push(e), &mut |_| {});

        assert_eq!(eng.applies, vec!["powersave"], "only the latest decision applies");
        assert_eq!(applied.len(), 1);
        assert!(applied[0].ok);
        assert_eq!(applied[0].profile, "powersave");
    }

    #[test]
    fn failed_apply_is_retried_once() {
        let (tx, rx) = mpsc::channel();
        tx.send(job("game")).unwrap();
        drop(tx);

        let mut eng = FakeEngine::with_script(vec![fail_out(false), ok_out(1)]);
        let mut applied = Vec::new();
        run(rx, &mut eng, Duration::from_millis(10), Duration::from_millis(5), &mut |e| applied.push(e), &mut |_| {});

        assert_eq!(eng.applies, vec!["game", "game"], "retry once");
        assert!(applied[0].ok, "final outcome after retry is ok");
    }

    #[test]
    fn engine_error_is_not_retried() {
        // hard error (e.g. unknown profile) must not be retried
        let (tx, rx) = mpsc::channel();
        tx.send(job("nope")).unwrap();
        drop(tx);

        let mut eng = FakeEngine::with_script(vec![fail_out(true)]);
        let mut applied = Vec::new();
        run(rx, &mut eng, Duration::from_millis(10), Duration::from_millis(5), &mut |e| applied.push(e), &mut |_| {});

        assert_eq!(eng.applies, vec!["nope"], "no retry for hard errors");
        assert!(!applied[0].ok);
    }

    #[test]
    fn already_active_skips_engine_apply() {
        let (tx, rx) = mpsc::channel();
        tx.send(job("balance")).unwrap();
        drop(tx);

        let mut eng = FakeEngine::default();
        eng.active = Some("balance".into());
        let mut applied = Vec::new();
        run(rx, &mut eng, Duration::from_millis(10), Duration::from_millis(5), &mut |e| applied.push(e), &mut |_| {});

        assert!(eng.applies.is_empty(), "no engine call when already active");
        assert!(applied[0].ok);
        assert_eq!(applied[0].wrote, 0);
    }

    #[test]
    fn restore_in_window_cancels_pending_apply() {
        let (tx, rx) = mpsc::channel();
        tx.send(job("game")).unwrap();
        tx.send(Work::Restore { retire: false }).unwrap();
        drop(tx);

        let mut eng = FakeEngine::with_script(vec![ok_out(1)]);
        let mut applied = Vec::new();
        let mut restored = Vec::new();
        run(
            rx,
            &mut eng,
            Duration::from_millis(30),
            Duration::from_millis(5),
            &mut |e| applied.push(e),
            &mut |r| restored.push(r),
        );

        assert!(eng.applies.is_empty(), "the pending apply must be dropped");
        assert_eq!(eng.restores, 1, "restore runs");
        assert_eq!(eng.holds_released, 1, "holds released exactly once");
        assert_eq!(restored.len(), 1);
        assert!(restored[0].ok);
        assert!(applied.is_empty());
    }

    #[test]
    fn restore_after_apply_uses_clean_channel() {
        let (tx, rx) = mpsc::channel();
        tx.send(Work::Restore { retire: true }).unwrap();
        drop(tx);

        let mut eng = FakeEngine::default();
        let mut restored = Vec::new();
        run(rx, &mut eng, Duration::from_millis(10), Duration::from_millis(5), &mut |_| {}, &mut |r| restored.push(r));

        assert_eq!(eng.restores, 1);
        assert!(restored[0].retire);
    }

    #[test]
    fn saver_reason_label_is_translated() {
        assert_eq!(reason_text(true, "base"), "MIUI saver");
        assert_eq!(reason_text(false, "base"), "base");
        assert_eq!(reason_text(true, "app"), "app");
    }
}
