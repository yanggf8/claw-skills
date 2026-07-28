use claw_core::marker::{emit_fallback, emit_skill_status, emit_trace, parse_status, SkillStatus};

/// Tests mutate process-global environment variables. `--test-threads=1` is
/// documented but NOT enforced by the code, so a plain `cargo test` would race
/// and produce passes for the wrong reason. This lock makes the suite correct
/// regardless of how it is invoked.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn env_guard() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn with_job_id<T>(value: Option<&str>, f: impl FnOnce() -> T) -> T {
    let _guard = env_guard();
    match value {
        Some(v) => std::env::set_var("NULLCLAW_JOB_ID", v),
        None => std::env::remove_var("NULLCLAW_JOB_ID"),
    }
    let r = f();
    std::env::remove_var("NULLCLAW_JOB_ID");
    r
}

fn capture(f: impl FnOnce(&mut Vec<u8>)) -> String {
    let mut buf = Vec::new();
    f(&mut buf);
    String::from_utf8(buf).unwrap()
}

#[test]
fn status_noop_when_job_id_unset() {
    let out = with_job_id(None, || capture(|b| emit_skill_status(SkillStatus::Ok, b).unwrap()));
    assert_eq!(out, "");
}

#[test]
fn status_emits_when_job_id_set() {
    let out = with_job_id(Some("job-123:7"), || {
        capture(|b| emit_skill_status(SkillStatus::Ok, b).unwrap())
    });
    assert_eq!(out, "[skill-status:ok]\n");
}

#[test]
fn status_emits_degraded() {
    let out = with_job_id(Some("job-123:7"), || {
        capture(|b| emit_skill_status(SkillStatus::Degraded, b).unwrap())
    });
    assert_eq!(out, "[skill-status:degraded]\n");
}

#[test]
fn status_emits_failed() {
    let out = with_job_id(Some("job-123:7"), || {
        capture(|b| emit_skill_status(SkillStatus::Failed, b).unwrap())
    });
    assert_eq!(out, "[skill-status:failed]\n");
}

#[test]
fn trace_noop_when_job_id_unset() {
    let out = with_job_id(None, || capture(|b| emit_trace(b).unwrap()));
    assert_eq!(out, "");
}

#[test]
fn trace_emits_exact_job_id() {
    // nullclaw compares the marker payload to the run trace id with mem.eql,
    // so this must be byte-exact — no trimming, no normalisation.
    let out = with_job_id(Some("job-abc:42"), || capture(|b| emit_trace(b).unwrap()));
    assert_eq!(out, "[trace:job-abc:42]\n");
}

#[test]
fn parse_status_rejects_unknown() {
    assert_eq!(parse_status("ok"), Some(SkillStatus::Ok));
    assert_eq!(parse_status("degraded"), Some(SkillStatus::Degraded));
    assert_eq!(parse_status("failed"), Some(SkillStatus::Failed));
    assert_eq!(parse_status("bogus"), None);
    assert_eq!(parse_status("OK"), None, "matching is case-sensitive");
}

#[test]
fn fallback_always_emits_and_punctuates_by_elapsed() {
    // Never job-id gated — manual runs must stay diagnosable.
    let with_ms = with_job_id(None, || {
        capture(|b| emit_fallback("weather", "CWA", "HKO", "CWA returned HTTP 502", "the Taipei forecast", Some(1200), b).unwrap())
    });
    assert_eq!(
        with_ms,
        "[skill-event] weather skill fell back from CWA to HKO because CWA returned HTTP 502. Fallback covered the Taipei forecast and took 1200ms.\n"
    );

    let without_ms = with_job_id(None, || {
        capture(|b| emit_fallback("weather", "CWA", "HKO", "CWA returned HTTP 502", "the Taipei forecast", None, b).unwrap())
    });
    assert_eq!(
        without_ms,
        "[skill-event] weather skill fell back from CWA to HKO because CWA returned HTTP 502. Fallback covered the Taipei forecast.\n"
    );
}
