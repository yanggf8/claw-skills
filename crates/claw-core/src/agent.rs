//! Call the nullclaw agent subprocess for LLM advice (e.g. clothing).
//!
//! Port of `weather/scripts/run.py:210-236` (`clothing_advice_llm`).
//! Binary path is resolved through `$HOME` — that is the shared injection
//! seam the differential harness depends on. Do not replace it with a
//! constant or a bespoke env var.

use crate::sanitize::strip_agent_artifacts;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// `<HOME>/nullclaw/zig-out/bin/nullclaw` — same as Python `os.path.expanduser`.
pub fn agent_binary_path() -> PathBuf {
    let home = std::env::var_os("HOME").unwrap_or_default();
    PathBuf::from(home).join("nullclaw/zig-out/bin/nullclaw")
}

/// Run `nullclaw agent -m <prompt>` and return sanitized stdout.
///
/// Empty string means "no advice". Exit code is ignored (B13). Spawn errors
/// and timeouts print a WARN to stderr and return empty.
pub fn call_agent(prompt: &str, timeout: Duration) -> String {
    match call_agent_inner(prompt, timeout) {
        Ok(advice) => advice,
        Err(e) => {
            eprintln!("[WARN] LLM clothing advice failed: {e}");
            String::new()
        }
    }
}

fn call_agent_inner(prompt: &str, timeout: Duration) -> Result<String, String> {
    let bin = agent_binary_path();
    let mut child = Command::new(&bin)
        .args(["agent", "-m", prompt])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;

    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => break,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("timed out after {timeout:?}"));
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => return Err(e.to_string()),
        }
    }

    // B13: exit code is ignored — read stdout regardless of returncode.
    let mut stdout = String::new();
    if let Some(mut out) = child.stdout.take() {
        out.read_to_string(&mut stdout).map_err(|e| e.to_string())?;
    }

    Ok(strip_agent_artifacts(&stdout, true))
}
