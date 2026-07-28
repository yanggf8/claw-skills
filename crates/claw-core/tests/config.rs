use claw_core::config::{get_bot_token, resolve_config_path};
use std::io::Write;
use std::path::PathBuf;

/// Process-global env mutation — every test takes the lock so a plain
/// `cargo test` (multi-threaded) cannot race on `CLAW_CONFIG`.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn guard() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn write_tmp(name: &str, body: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    // One file per CALL, not per process. Sharing one path meant File::create
    // truncated it while another thread was reading.
    static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    p.push(format!("claw-core-cfg-{name}-{}-{}.json", std::process::id(), n));
    let mut f = std::fs::File::create(&p).unwrap();
    f.write_all(body.as_bytes()).unwrap();
    p
}

#[test]
fn explicit_path_wins_over_env() {
    let _g = guard();
    let explicit = write_tmp("explicit", r#"{"channels":{"telegram":{"botToken":"EXPLICIT"}}}"#);
    let other = write_tmp("env", r#"{"channels":{"telegram":{"botToken":"ENV"}}}"#);
    std::env::set_var("CLAW_CONFIG", &other);
    assert_eq!(get_bot_token("main", Some(&explicit)).as_deref(), Some("EXPLICIT"));
    assert_eq!(resolve_config_path(Some(&explicit)), explicit);
    std::env::remove_var("CLAW_CONFIG");
}

#[test]
fn nullclaw_account_schema_preferred() {
    let _g = guard();
    let p = write_tmp(
        "nullclaw",
        r#"{"channels":{"telegram":{"accounts":{"main":{"bot_token":"ACCT"}},"botToken":"SINGLE"}}}"#,
    );
    assert_eq!(get_bot_token("main", Some(&p)).as_deref(), Some("ACCT"));
}

#[test]
fn falls_back_to_single_token_when_account_absent() {
    let _g = guard();
    // Mixed-schema file, requested account missing: Python still falls through
    // to botToken rather than failing. Preserve that.
    let p = write_tmp(
        "mixed",
        r#"{"channels":{"telegram":{"accounts":{"other":{"bot_token":"ACCT"}},"botToken":"SINGLE"}}}"#,
    );
    assert_eq!(get_bot_token("main", Some(&p)).as_deref(), Some("SINGLE"));
}

#[test]
fn missing_file_is_none_not_panic() {
    let _g = guard();
    let p = PathBuf::from("/nonexistent/claw-core/definitely-not-here.json");
    assert_eq!(get_bot_token("main", Some(&p)), None);
}

#[test]
fn malformed_json_is_none_not_panic() {
    let _g = guard();
    let p = write_tmp("malformed", "{ this is not json");
    assert_eq!(get_bot_token("main", Some(&p)), None);
}

#[test]
fn empty_token_treated_as_absent() {
    let _g = guard();
    let p = write_tmp("empty", r#"{"channels":{"telegram":{"accounts":{"main":{"bot_token":""}},"botToken":"SINGLE"}}}"#);
    assert_eq!(get_bot_token("main", Some(&p)).as_deref(), Some("SINGLE"));
}

#[test]
fn no_telegram_section_is_none() {
    let _g = guard();
    let p = write_tmp("bare", r#"{"channels":{}}"#);
    assert_eq!(get_bot_token("main", Some(&p)), None);
}

#[test]
fn claw_config_env_var_is_used_when_no_explicit_path() {
    let _g = guard();
    let p = write_tmp("via-env", r#"{"channels":{"telegram":{"botToken":"FROM_ENV"}}}"#);
    std::env::set_var("CLAW_CONFIG", &p);
    assert_eq!(get_bot_token("main", None).as_deref(), Some("FROM_ENV"));
    assert_eq!(resolve_config_path(None), p);
    std::env::remove_var("CLAW_CONFIG");
}
