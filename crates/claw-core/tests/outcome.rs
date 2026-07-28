use claw_core::marker::SkillStatus;
use claw_core::outcome::{finish, Finish};

static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn with_job_id<T>(v: Option<&str>, f: impl FnOnce() -> T) -> T {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    match v {
        Some(v) => std::env::set_var("NULLCLAW_JOB_ID", v),
        None => std::env::remove_var("NULLCLAW_JOB_ID"),
    }
    let r = f();
    std::env::remove_var("NULLCLAW_JOB_ID");
    r
}

#[test]
fn marked_ok_emits_status_then_trace_in_that_order() {
    let mut out = Vec::new();
    let code = with_job_id(Some("t-1"), || {
        finish(Finish::Marked { status: SkillStatus::Ok, exit: 0 }, &mut out)
    });
    assert_eq!(code, 0);
    assert_eq!(String::from_utf8(out).unwrap(), "[skill-status:ok]\n[trace:t-1]\n");
}

#[test]
fn marked_degraded_still_exits_zero() {
    // degraded is a SEMANTIC status, not a process failure. nullclaw turns it
    // into verified=2 / last_status=error on its own.
    let mut out = Vec::new();
    let code = with_job_id(Some("t-2"), || {
        finish(Finish::Marked { status: SkillStatus::Degraded, exit: 0 }, &mut out)
    });
    assert_eq!(code, 0);
    assert_eq!(String::from_utf8(out).unwrap(), "[skill-status:degraded]\n[trace:t-2]\n");
}

#[test]
fn unmarked_emits_nothing_and_returns_exit() {
    let mut out = Vec::new();
    let code = with_job_id(Some("t-3"), || finish(Finish::Unmarked { exit: 1 }, &mut out));
    assert_eq!(code, 1);
    assert_eq!(String::from_utf8(out).unwrap(), "");
}

#[test]
fn marked_is_silent_without_job_id() {
    let mut out = Vec::new();
    let code = with_job_id(None, || {
        finish(Finish::Marked { status: SkillStatus::Ok, exit: 0 }, &mut out)
    });
    assert_eq!(code, 0);
    assert_eq!(String::from_utf8(out).unwrap(), "");
}
