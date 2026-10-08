//! Shared integration-test harness: spawn the real `miui-ft serve` binary and
//! drive it over JSON-lines. Each test binary uses a subset of these helpers.

#![allow(dead_code)] // a subset is used per test binary

use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

pub struct Daemon {
    pub child: Child,
    pub stdin: ChildStdin,
    pub stdout: BufReader<ChildStdout>,
}

impl Daemon {
    pub fn spawn(state_dir: &Path, config: &Path) -> Self {
        Self::spawn_env(state_dir, config, &[])
    }

    /// Spawn with extra environment (fake logcat/settings/sysfs overrides).
    /// Daemon stderr is captured in `<state_dir>/daemon-stderr.log`.
    pub fn spawn_env(state_dir: &Path, config: &Path, envs: &[(&str, &str)]) -> Self {
        let bin = env!("CARGO_BIN_EXE_miui-ft");
        let _ = std::fs::create_dir_all(state_dir);
        let log = std::fs::File::create(state_dir.join("daemon-stderr.log")).ok();
        let mut cmd = Command::new(bin);
        cmd.args([
            "serve",
            "--state-dir",
            state_dir.to_str().unwrap(),
            "--config",
            config.to_str().unwrap(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(log.map(Stdio::from).unwrap_or_else(Stdio::null));
        for (k, v) in envs {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().expect("spawn daemon");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Daemon {
            child,
            stdin,
            stdout,
        }
    }

    pub fn send(&mut self, v: Value) {
        writeln!(self.stdin, "{v}").unwrap();
        self.stdin.flush().unwrap();
    }

    /// Reads events until `pred` matches (or timeout). Returns the event.
    pub fn wait_for(&mut self, pred: impl Fn(&Value) -> bool, timeout: Duration) -> Value {
        let deadline = Instant::now() + timeout;
        let mut line = String::new();
        loop {
            assert!(
                Instant::now() < deadline,
                "timeout waiting for event; last line: {line}"
            );
            line.clear();
            if self.stdout.read_line(&mut line).unwrap_or(0) == 0 {
                panic!("daemon stdout closed unexpectedly");
            }
            let v: Value = serde_json::from_str(line.trim()).expect("daemon line is JSON");
            if pred(&v) {
                return v;
            }
        }
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.stdin.write_all(b"{\"cmd\":\"shutdown\"}\n");
        let _ = self.stdin.flush();
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if let Ok(Some(_)) = self.child.try_wait() {
                return;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub trait WaitTimeout {
    fn wait_timeout(&mut self, d: Duration) -> std::io::Result<Option<std::process::ExitStatus>>;
}

impl WaitTimeout for Child {
    fn wait_timeout(&mut self, d: Duration) -> std::io::Result<Option<std::process::ExitStatus>> {
        let deadline = Instant::now() + d;
        loop {
            if let Some(st) = self.try_wait()? {
                return Ok(Some(st));
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }
}

/// Fresh temp dir per test (process-id + tag tagged, recreated on entry).
pub fn tmp(tag: &str) -> PathBuf {
    let mut d = std::env::temp_dir();
    d.push(format!("mifinetune-test-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}
