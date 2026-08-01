//! Running the nullclaw agent, and deciding when to try again.
//!
//! `claw_core::agent::call_agent` is not enough here: it returns sanitised
//! stdout and swallows the exit code, and this skill's retry rule turns
//! entirely on distinguishing "the provider stalled" from "the model answered
//! something unusable".

use crate::config::llm_retry_timeout_secs;
use crate::select::NumberedMap;
use crate::trace::{clip_subprocess_text, log_trace};
use claw_core::budget::monotonic_secs;
use serde_json::json;
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// A synthetic code, not one the agent can return: the process was killed for
/// running past its budget.
pub const TIMEOUT_RC: i32 = 124;

#[derive(Debug, Clone)]
pub struct AgentResult {
    pub returncode: i32,
    pub stdout: String,
    pub stderr: String,
}

impl AgentResult {
    pub fn timed_out(&self) -> bool {
        self.returncode == TIMEOUT_RC
    }
    pub fn usable(&self) -> bool {
        self.returncode == 0 && !self.stdout.trim().is_empty()
    }
}

fn agent_binary() -> std::path::PathBuf {
    crate::config::home().join("nullclaw/zig-out/bin/nullclaw")
}

fn source_item_counts(all_items: &[(String, usize)]) -> serde_json::Value {
    let mut m = serde_json::Map::new();
    for (label, n) in all_items {
        m.insert(label.clone(), json!(n));
    }
    serde_json::Value::Object(m)
}

/// Remaining wall clock for a retry, from the scheduler's env.
///
/// `None` means no budget is configured — a manual run — and the caller then
/// treats a retry as always permitted. Two seconds of headroom are reserved so
/// the retry cannot itself overrun the skill's hard kill.
///
/// `NULLCLAW_SKILL_STARTED` is CLOCK_MONOTONIC seconds, the same domain as
/// `monotonic_secs`. A wall-clock value would make the subtraction hugely
/// negative, the clamp would turn that into zero elapsed, and the budget would
/// silently do nothing while looking healthy.
pub fn llm_retry_budget_secs() -> Option<f64> {
    let timeout: f64 = std::env::var("NULLCLAW_SKILL_TIMEOUT")
        .ok()
        .filter(|s| !s.is_empty())?
        .parse()
        .ok()?;
    if timeout <= 0.0 {
        return None;
    }
    match std::env::var("NULLCLAW_SKILL_STARTED")
        .ok()
        .filter(|s| !s.is_empty())
        .and_then(|s| s.parse::<f64>().ok())
    {
        Some(started) => {
            let elapsed = (monotonic_secs() - started).max(0.0);
            Some((timeout - elapsed - 2.0).max(0.0))
        }
        None => Some((timeout - 2.0).max(0.0)),
    }
}

/// `(elapsed since skill start, remaining before kill)` — for timing traces
/// only.
///
/// Deliberately separate from [`llm_retry_budget_secs`] so that function's
/// two-second reserve stays intact. Both are `None` when the scheduler cannot
/// supply them: elapsed needs the start time, and remaining needs a valid
/// timeout *and* a known elapsed, because a timeout alone says nothing about
/// how close the kill is.
pub fn skill_wallclock() -> (Option<f64>, Option<f64>) {
    let elapsed = std::env::var("NULLCLAW_SKILL_STARTED")
        .ok()
        .filter(|s| !s.is_empty())
        .and_then(|s| s.parse::<f64>().ok())
        .map(|started| (monotonic_secs() - started).max(0.0));

    let remaining = match (
        std::env::var("NULLCLAW_SKILL_TIMEOUT")
            .ok()
            .filter(|s| !s.is_empty())
            .and_then(|s| s.parse::<f64>().ok()),
        elapsed,
    ) {
        (Some(t), Some(e)) if t > 0.0 => Some((t - e).max(0.0)),
        _ => None,
    };
    (elapsed, remaining)
}

/// One call, retried once if — and only if — the provider stalled.
///
/// Validation failures, empty stdout, and other non-zero exits are
/// deterministic: the same prompt will produce the same non-answer, so a retry
/// spends the budget for nothing. The retry is also skipped when what remains
/// of the cron budget cannot fit it, so a wedged provider cannot push a
/// multi-topic run past its kill window.
pub fn run_agent(
    prompt: &str,
    timeout_secs: u64,
    variant: &str,
    counts: &[(String, usize)],
    numbered: &NumberedMap,
) -> AgentResult {
    let result = run_agent_once(prompt, timeout_secs, variant, counts, numbered);
    if !result.timed_out() {
        return result;
    }

    let retry_timeout = llm_retry_timeout_secs().min(timeout_secs);
    if let Some(budget) = llm_retry_budget_secs() {
        if budget < retry_timeout as f64 {
            log_trace(
                "llm_agent_retry_skipped_budget",
                json!({"variant": variant,
                       "budget_secs": (budget * 10.0).round() / 10.0,
                       "retry_timeout": retry_timeout}),
            );
            return result;
        }
    }

    log_trace(
        "llm_agent_retry",
        json!({"variant": variant, "attempt": 2, "first_timeout": timeout_secs,
               "retry_timeout": retry_timeout}),
    );
    run_agent_once(prompt, retry_timeout, variant, counts, numbered)
}

/// The single-shot call, for the one caller that must not retry.
pub fn run_agent_once_public(
    prompt: &str,
    timeout_secs: u64,
    variant: &str,
    counts: &[(String, usize)],
    numbered: &NumberedMap,
) -> AgentResult {
    run_agent_once(prompt, timeout_secs, variant, counts, numbered)
}

fn run_agent_once(
    prompt: &str,
    timeout_secs: u64,
    variant: &str,
    counts: &[(String, usize)],
    numbered: &NumberedMap,
) -> AgentResult {
    log_trace(
        "llm_agent_start",
        json!({"variant": variant, "timeout_secs": timeout_secs,
               "source_item_counts": source_item_counts(counts),
               "items_numbered": numbered.len(),
               "prompt_chars": prompt.chars().count()}),
    );

    let started = Instant::now();
    let spawned = Command::new(agent_binary())
        .args(["agent", "--isolated", "-m", prompt])
        .env("NULLCLAW_AGENT_TIMING_TRACE", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();

    let mut child = match spawned {
        Ok(c) => c,
        Err(e) => {
            // A missing binary is not a timeout and must not be retried as one.
            log_trace(
                "llm_agent_spawn_error",
                json!({"variant": variant, "error": e.to_string()}),
            );
            return AgentResult {
                returncode: 127,
                stdout: String::new(),
                stderr: e.to_string(),
            };
        }
    };

    // stdout and stderr are drained on their own threads. A pipe left unread
    // fills, the child blocks writing to it, and the wait below would time out
    // a model that was answering perfectly well.
    //
    // They append into shared buffers rather than returning a value, so a
    // timeout can take what arrived without joining. Joining would give the
    // timeout no force at all whenever the pipe outlives the process we
    // killed — a child that spawned its own child leaves the write end open,
    // and the read blocks until *that* exits, which is exactly the stalled
    // case the timeout exists for.
    let out_buf = Arc::new(Mutex::new(String::new()));
    let err_buf = Arc::new(Mutex::new(String::new()));
    let mut out_pipe = child.stdout.take();
    let mut err_pipe = child.stderr.take();
    let out_handle = {
        let buf = Arc::clone(&out_buf);
        std::thread::spawn(move || read_into(&mut out_pipe, &buf))
    };
    let err_handle = {
        let buf = Arc::clone(&err_buf);
        std::thread::spawn(move || read_into(&mut err_pipe, &buf))
    };

    let deadline = started + Duration::from_secs(timeout_secs);
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
            Err(e) => {
                log_trace(
                    "llm_agent_wait_error",
                    json!({"variant": variant, "error": e.to_string()}),
                );
                code = 1;
                break;
            }
        }
    }

    let snapshot = |b: &Arc<Mutex<String>>| b.lock().map(|g| g.clone()).unwrap_or_default();
    if timed_out {
        // Deliberately not joined: see the note above the readers.
        let (stdout, stderr) = (snapshot(&out_buf), snapshot(&err_buf));
        let elapsed_ms = started.elapsed().as_millis() as u64;

        log_trace(
            "llm_agent_timeout",
            json!({"variant": variant, "timeout_secs": timeout_secs,
                   "elapsed_ms": elapsed_ms,
                   "source_item_counts": source_item_counts(counts),
                   "items_numbered": numbered.len(),
                   "prompt_chars": prompt.chars().count(),
                   "stdout_len": stdout.chars().count(),
                   "stderr_len": stderr.chars().count(),
                   "stdout_tail": clip_subprocess_text(&stdout, 4000),
                   "stderr_tail": clip_subprocess_text(&stderr, 4000)}),
        );
        return AgentResult {
            returncode: TIMEOUT_RC,
            stdout: clip_subprocess_text(&stdout, 10000),
            stderr: clip_subprocess_text(&stderr, 10000),
        };
    }

    // The process exited on its own, so both pipes are closed and joining is
    // bounded — take the complete output rather than a snapshot.
    let _ = out_handle.join();
    let _ = err_handle.join();
    let (stdout, stderr) = (snapshot(&out_buf), snapshot(&err_buf));
    let elapsed_ms = started.elapsed().as_millis() as u64;

    log_trace(
        "llm_agent_exit",
        json!({"variant": variant, "elapsed_ms": elapsed_ms, "returncode": code,
               "stdout_len": stdout.chars().count(),
               "stderr_len": stderr.chars().count(),
               "stderr_tail": clip_subprocess_text(&stderr, 4000)}),
    );
    AgentResult {
        returncode: code,
        stdout,
        stderr,
    }
}

/// Read to EOF, appending as it goes so a caller that gives up still sees
/// whatever the child managed to say.
fn read_into(pipe: &mut Option<impl Read>, out: &Arc<Mutex<String>>) {
    let Some(p) = pipe.as_mut() else {
        return;
    };
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
