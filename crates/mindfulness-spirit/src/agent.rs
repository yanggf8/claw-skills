//! Running the nullclaw agent for the writer and checklist passes.
//!
//! `claw_core::agent::call_agent` is not enough: it discards the exit code, and
//! both callers here branch on it — a stall and a refusal need different
//! handling, and neither may be mistaken for a draft.

use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Synthetic: the process was killed for running past its budget.
pub const TIMEOUT_RC: i32 = 124;
/// Each pass writes or reviews a 2000–3500 character article.
pub const TIMEOUT: Duration = Duration::from_secs(300);
/// What is kept of a killed run's output, for the diagnostic.
const CLIP: usize = 10_000;

#[derive(Debug, Clone)]
pub struct Output {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    pub fn timed_out(&self) -> bool {
        self.code == TIMEOUT_RC
    }

    /// The sentence that goes in front of an operator.
    pub fn failure_reason(&self) -> String {
        if self.timed_out() {
            return format!("timed out after {}s", TIMEOUT.as_secs());
        }
        let stderr = self.stderr.trim();
        if stderr.is_empty() {
            format!("exit code {}", self.code)
        } else {
            format!(
                "exit code {}: {}",
                self.code,
                stderr.chars().take(500).collect::<String>()
            )
        }
    }
}

pub fn binary() -> std::path::PathBuf {
    crate::config::home().join("nullclaw/zig-out/bin/nullclaw")
}

pub fn run(prompt: &str, timeout: Duration) -> Output {
    let spawned = Command::new(binary())
        .args(["agent", "--isolated", "-m", prompt])
        .env("NULLCLAW_AGENT_TIMING_TRACE", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();

    let mut child = match spawned {
        Ok(c) => c,
        Err(e) => {
            return Output {
                code: 127,
                stdout: String::new(),
                stderr: format!("could not run {}: {e}", binary().display()),
            }
        }
    };

    // Drained on their own threads into shared buffers. A pipe left unread
    // fills and blocks the child; and reading by joining after a kill gives
    // the timeout no force at all when the child left a grandchild holding
    // the write end — which is exactly the stalled case it exists for.
    let out_buf = Arc::new(Mutex::new(String::new()));
    let err_buf = Arc::new(Mutex::new(String::new()));
    let mut op = child.stdout.take();
    let mut ep = child.stderr.take();
    let oh = {
        let b = Arc::clone(&out_buf);
        std::thread::spawn(move || drain(&mut op, &b))
    };
    let eh = {
        let b = Arc::clone(&err_buf);
        std::thread::spawn(move || drain(&mut ep, &b))
    };

    let deadline = Instant::now() + timeout;
    let mut timed_out = false;
    let mut code = 0;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                code = status.code().unwrap_or(1);
                break;
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    timed_out = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(_) => {
                code = 1;
                break;
            }
        }
    }

    let snapshot = |b: &Arc<Mutex<String>>| b.lock().map(|g| g.clone()).unwrap_or_default();
    if timed_out {
        return Output {
            code: TIMEOUT_RC,
            stdout: clip(&snapshot(&out_buf)),
            stderr: clip(&snapshot(&err_buf)),
        };
    }
    let _ = oh.join();
    let _ = eh.join();
    Output {
        code,
        stdout: snapshot(&out_buf),
        stderr: snapshot(&err_buf),
    }
}

fn clip(s: &str) -> String {
    s.chars().take(CLIP).collect()
}

fn drain(pipe: &mut Option<impl Read>, out: &Arc<Mutex<String>>) {
    let Some(p) = pipe.as_mut() else { return };
    let mut chunk = [0u8; 8192];
    loop {
        match p.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(n) => {
                let text = String::from_utf8_lossy(&chunk[..n]).into_owned();
                if let Ok(mut g) = out.lock() {
                    g.push_str(&text);
                }
            }
        }
    }
}
