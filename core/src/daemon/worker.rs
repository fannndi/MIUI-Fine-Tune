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
    Restore {
        retire: bool,
    },
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
        Outcome {
            ok: false,
            wrote: 0,
            verified: 0,
            failed: 0,
            error: Some(msg),
            already: false,
        }
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
                on_restored(RestoredEvent {
                    retire,
                    ok: out.ok,
                    wrote: out.wrote,
                    verified: out.verified,
                    failed: out.failed,
                });
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
                    on_restored(RestoredEvent {
                        retire: r.retire,
                        ok: out.ok,
                        wrote: out.wrote,
                        verified: out.verified,
                        failed: out.failed,
                    });
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
        return Outcome {
            ok: true,
            wrote: 0,
            verified: 0,
            failed: 0,
            error: None,
            already: true,
        };
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
mod tests;
