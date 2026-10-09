use super::*;
use std::sync::mpsc;

fn ok_out(wrote: usize) -> Outcome {
    Outcome {
        ok: true,
        wrote,
        verified: wrote,
        failed: 0,
        error: None,
        already: false,
    }
}

fn fail_out(errored: bool) -> Outcome {
    Outcome {
        ok: false,
        wrote: 1,
        verified: 0,
        failed: 1,
        error: errored.then(|| "boom".into()),
        already: false,
    }
}

/// Scriptable engine: records calls, returns scripted outcomes.
#[derive(Default)]
struct FakeEngine {
    active: Option<String>,
    applies: Vec<String>,
    reconciles: Vec<bool>,
    restores: usize,
    holds_released: usize,
    /// Outcomes returned in order; the last one repeats.
    script: Vec<Outcome>,
}

impl FakeEngine {
    fn with_script(script: Vec<Outcome>) -> Self {
        FakeEngine {
            reconciles: Vec::new(),
            script,
            ..Default::default()
        }
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
    fn apply(&mut self, profile_id: &str, reconcile: bool) -> Outcome {
        self.reconciles.push(reconcile);
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
        force: false,
    })
}

/// Same as [`job`] but with the force flag (config/pack/user-tap path).
fn forced_job(profile: &str) -> Work {
    match job(profile) {
        Work::Apply(mut j) => {
            j.force = true;
            Work::Apply(j)
        }
        w => w,
    }
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
    run(
        rx,
        &mut eng,
        Duration::from_millis(60),
        Duration::from_millis(5),
        &mut |e| applied.push(e),
        &mut |_| {},
    );

    assert_eq!(
        eng.applies,
        vec!["powersave"],
        "only the latest decision applies"
    );
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
    run(
        rx,
        &mut eng,
        Duration::from_millis(10),
        Duration::from_millis(5),
        &mut |e| applied.push(e),
        &mut |_| {},
    );

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
    run(
        rx,
        &mut eng,
        Duration::from_millis(10),
        Duration::from_millis(5),
        &mut |e| applied.push(e),
        &mut |_| {},
    );

    assert_eq!(eng.applies, vec!["nope"], "no retry for hard errors");
    assert!(!applied[0].ok);
}

#[test]
fn already_active_skips_engine_apply() {
    let (tx, rx) = mpsc::channel();
    tx.send(job("balance")).unwrap();
    drop(tx);

    let mut eng = FakeEngine {
        active: Some("balance".into()),
        ..Default::default()
    };
    let mut applied = Vec::new();
    run(
        rx,
        &mut eng,
        Duration::from_millis(10),
        Duration::from_millis(5),
        &mut |e| applied.push(e),
        &mut |_| {},
    );

    assert!(eng.applies.is_empty(), "no engine call when already active");
    assert!(applied[0].ok);
    assert_eq!(applied[0].wrote, 0);
}

#[test]
fn forced_job_replans_even_when_already_active() {
    // the pack-update path: same profile id, but the engine must run so new
    // catalog keys / drift are reconciled
    let (tx, rx) = mpsc::channel();
    tx.send(forced_job("balance")).unwrap();
    drop(tx);

    let mut eng = FakeEngine {
        active: Some("balance".into()),
        ..Default::default()
    };
    let mut applied = Vec::new();
    run(
        rx,
        &mut eng,
        Duration::from_millis(10),
        Duration::from_millis(5),
        &mut |e| applied.push(e),
        &mut |_| {},
    );

    assert_eq!(
        eng.applies,
        vec!["balance"],
        "forced job must reach the engine"
    );
    assert!(applied[0].ok);
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
    run(
        rx,
        &mut eng,
        Duration::from_millis(10),
        Duration::from_millis(5),
        &mut |_| {},
        &mut |r| restored.push(r),
    );

    assert_eq!(eng.restores, 1);
    assert!(restored[0].retire);
}

#[test]
fn saver_reason_label_is_translated() {
    assert_eq!(reason_text(true, "base"), "MIUI saver");
    assert_eq!(reason_text(false, "base"), "base");
    assert_eq!(reason_text(true, "app"), "app");
}

#[test]
fn job_force_reaches_engine_as_reconcile() {
    let (tx, rx) = mpsc::channel();
    tx.send(forced_job("balance")).unwrap();
    drop(tx);

    let mut eng = FakeEngine::default();
    let mut applied = Vec::new();
    run(
        rx,
        &mut eng,
        Duration::from_millis(10),
        Duration::from_millis(5),
        &mut |e| applied.push(e),
        &mut |_| {},
    );
    assert_eq!(eng.reconciles, vec![true]);
    assert!(applied[0].ok);
}
