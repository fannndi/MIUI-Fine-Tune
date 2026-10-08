//! `serve` — daemon entry point (F0 connectivity spike).
//!
//! Protocol: JSON-lines over stdio.
//!   stdin  <- app commands   e.g. `{"cmd":"ping"}`
//!   stdout -> daemon events  e.g. `{"event":"pong"}`
//!   stderr -> human logs     (the app relays these to logcat)
//!
//! Spike goals (F0), verified on device:
//!   1. `su -c` forwards stdin to the daemon (the one unknown).
//!   2. stdout is line-buffered: events stream in real time, not at exit.
//!   3. The daemon can push spontaneous events (state changes) anytime.
//!   4. stdin EOF (app died) makes the daemon exit cleanly.
//!
//! This file is replaced by the real `daemon/` modules in F2.

use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

static DONE: AtomicBool = AtomicBool::new(false);

/// Writes one JSON line to stdout and flushes (line protocol requires it).
fn emit(line: &str) {
    let mut out = std::io::stdout();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

pub fn run() -> Result<(), String> {
    eprintln!("log: serve up (pid={})", std::process::id());

    // Spontaneous heartbeat: proves the daemon can push while stdin is idle.
    std::thread::spawn(|| {
        let mut n = 0u32;
        while !DONE.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_secs(2));
            n += 1;
            emit(&format!(r#"{{"event":"tick","n":{n}}}"#));
        }
    });

    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                eprintln!("log: stdin read error: {e}");
                break;
            }
        };
        if line.contains("\"quit\"") {
            emit(r#"{"event":"bye"}"#);
            break;
        }
        let payload = serde_json::to_string(&line).unwrap_or_else(|_| "\"\"".into());
        emit(&format!(r#"{{"event":"echo","data":{payload}}}"#));
    }

    DONE.store(true, Ordering::Relaxed);
    eprintln!("log: stdin closed, exiting");
    Ok(())
}
