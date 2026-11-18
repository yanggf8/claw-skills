//! Structured diagnostics, one JSON object per line.
//!
//! Serialised behind a mutex. Some entries carry 4000-character stdout tails,
//! which exceed the write buffer — without the lock, concurrent cross-dedup
//! samples interleave mid-object and break the one-object-per-line property
//! every ops query depends on.

use crate::config::trace_file;
use serde_json::{json, Map, Value};
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

static TRACE_LOCK: Mutex<()> = Mutex::new(());

/// Set once by `news shadow-ai` in `main`, before any thread exists [R9].
static SHADOW_PROCESS: AtomicBool = AtomicBool::new(false);

/// Mark this whole process as the shadow: from now on every event
/// [`log_trace`] writes carries `"ai_sources": "shadow"`, whatever thread
/// writes it and however late — worker threads abandoned past a deadline
/// included, which is why this is a process-wide flag and not a scope.
///
/// There is no way to unset it. It belongs to the process entry point: the
/// library never sets it, so a delivering run is never stamped, and a test
/// binary that calls this is a shadow process for all of its tests.
pub fn stamp_process_as_shadow() {
    SHADOW_PROCESS.store(true, Ordering::SeqCst);
}

pub fn job_id() -> String {
    std::env::var("NULLCLAW_JOB_ID").unwrap_or_else(|_| "interactive".to_string())
}

/// Append one event. Never fails the run: a diagnostic that cannot be written
/// is a warning on stderr, not an error.
pub fn log_trace(event: &str, fields: Value) {
    let mut entry = Map::new();
    entry.insert(
        "ts".into(),
        json!(jiff::Timestamp::now().to_string()),
    );
    entry.insert("job_id".into(), json!(job_id()));
    entry.insert("skill".into(), json!("news"));
    entry.insert("event".into(), json!(event));
    if let Value::Object(extra) = fields {
        for (k, v) in extra {
            entry.insert(k, v);
        }
    }
    // After the event's own fields, so it wins: `ai_section_rendered` carries
    // its own `ai_sources`, and under the stamp the one key reads "shadow".
    if SHADOW_PROCESS.load(Ordering::SeqCst) {
        entry.insert("ai_sources".into(), json!("shadow"));
    }

    let line = match serde_json::to_string(&Value::Object(entry)) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[WARN] trace encode failed: {e}");
            return;
        }
    };

    let _guard = TRACE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let write = || -> std::io::Result<()> {
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(trace_file())?;
        writeln!(f, "{line}")
    };
    if let Err(e) = write() {
        eprintln!("[WARN] trace write failed: {e}");
    }
}

/// Trim a subprocess capture, then keep its first `limit` characters.
///
/// The trace fields that carry this are named `stdout_tail` / `stderr_tail`,
/// which is a misnomer — it has always been the head. Kept as-is: the field
/// names appear in recorded traces and ops queries, and changing what the
/// value means would silently reinterpret the history.
pub fn clip_subprocess_text(value: &str, limit: usize) -> String {
    value.trim().chars().take(limit).collect()
}

/// Up to `limit` non-blank lines, each cut to 240 characters.
pub fn sample_nonempty_lines(value: &str, limit: usize) -> Vec<String> {
    value
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .take(limit)
        .map(|l| l.chars().take(240).collect())
        .collect()
}
