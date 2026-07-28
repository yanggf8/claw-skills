mod support { pub mod stub_server; }
use claw_core::telegram::{send, SendOptions};
use std::io::Write;
use std::path::PathBuf;
use support::stub_server;

fn cfg_with_token() -> PathBuf {
    let mut p = std::env::temp_dir();
    // One file per CALL, not per process. Sharing one path meant File::create
    // truncated it while another thread's send() was reading, yielding an empty
    // file -> no token -> a silent zero-attempt "failure" in an unrelated test.
    static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    p.push(format!("claw-core-tg-{}-{}.json", std::process::id(), n));
    let mut f = std::fs::File::create(&p).unwrap();
    f.write_all(br#"{"channels":{"telegram":{"botToken":"T"}}}"#).unwrap();
    p
}

fn opts(base: &str) -> SendOptions {
    SendOptions {
        config_path: Some(cfg_with_token()),
        base_url: Some(base.to_string()),
        ..Default::default()
    }
}

#[test]
fn success_first_try() {
    let s = stub_server::start(vec![Some(200)], 0);
    assert!(send("chat", "hi", &opts(&s.base_url)));
    assert_eq!(s.attempts(), 1);
}

#[test]
fn parse_mode_default_included_and_none_omitted() {
    let s = stub_server::start(vec![Some(200), Some(200)], 0);
    let mut o = opts(&s.base_url);
    assert!(send("chat", "hi", &o));
    o.parse_mode = None;
    assert!(send("chat", "hi", &o));
    assert!(s.body(0).contains("\"parse_mode\":\"Markdown\""));
    assert!(!s.body(1).contains("parse_mode"), "key must be absent, not null");
    assert!(s.body(0).contains("\"disable_web_page_preview\":true"));
}

#[test]
fn retries_502_502_then_succeeds() {
    let s = stub_server::start(vec![Some(502), Some(502), Some(200)], 0);
    assert!(send("chat", "hi", &opts(&s.base_url)));
    assert_eq!(s.attempts(), 3);
}

#[test]
fn three_502_returns_false() {
    let s = stub_server::start(vec![Some(502), Some(502), Some(502)], 0);
    assert!(!send("chat", "hi", &opts(&s.base_url)));
    assert_eq!(s.attempts(), 3, "exactly 3 attempts, never 4");
}

#[test]
fn permanent_403_does_not_retry() {
    let s = stub_server::start(vec![Some(403)], 0);
    assert!(!send("chat", "hi", &opts(&s.base_url)));
    assert_eq!(s.attempts(), 1);
}

#[test]
fn http_408_is_permanent_not_retryable() {
    // Deliberate: Python's _is_retryable_http covers only 429 and 5xx.
    // 408 Request Timeout looks retryable but is NOT. Preserve it.
    let s = stub_server::start(vec![Some(408)], 0);
    assert!(!send("chat", "hi", &opts(&s.base_url)));
    assert_eq!(s.attempts(), 1);
}

#[test]
fn http_429_retries() {
    let s = stub_server::start(vec![Some(429), Some(200)], 0);
    assert!(send("chat", "hi", &opts(&s.base_url)));
    assert_eq!(s.attempts(), 2);
}

#[test]
fn non_200_2xx_is_failure_without_retry() {
    let s = stub_server::start(vec![Some(204)], 0);
    assert!(!send("chat", "hi", &opts(&s.base_url)));
    assert_eq!(s.attempts(), 1);
}

#[test]
fn success_does_not_require_ok_true_body() {
    // The stub returns Content-Length: 0. Python treats HTTP 200 as success
    // without parsing the body; Rust must not start requiring {"ok":true}.
    let s = stub_server::start(vec![Some(200)], 0);
    assert!(send("chat", "hi", &opts(&s.base_url)));
}

#[test]
fn zero_deadline_skips_entirely() {
    let s = stub_server::start(vec![Some(200)], 0);
    let mut o = opts(&s.base_url);
    o.deadline_s = Some(0.0);
    assert!(!send("chat", "hi", &o));
    assert_eq!(s.attempts(), 0, "no HTTP attempt when the budget is already spent");
}

#[test]
fn deadline_blocks_second_attempt() {
    let s = stub_server::start(vec![None, Some(200)], 300);
    let mut o = opts(&s.base_url);
    o.deadline_s = Some(0.25);
    assert!(!send("chat", "hi", &o));
    assert_eq!(s.attempts(), 1, "budget exhausted after the first slow attempt");
}

#[test]
fn backoff_schedule_is_two_then_five_seconds() {
    // The gap between attempts is BACKOFFS_S, not a fixed or exponential delay.
    // Asserted on elapsed wall time with generous slack so a loaded machine does
    // not make this flaky, but tight enough to catch a wrong schedule.
    let s = stub_server::start(vec![Some(502), Some(502), Some(200)], 0);
    let mut o = opts(&s.base_url);
    o.deadline_s = Some(30.0);
    let t0 = std::time::Instant::now();
    assert!(send("chat", "hi", &o));
    let elapsed = t0.elapsed().as_secs_f64();
    assert_eq!(s.attempts(), 3);
    assert!(elapsed >= 7.0, "expected >= 2s + 5s of backoff, got {elapsed:.2}s");
    assert!(elapsed < 12.0, "backoff took far longer than 2s + 5s: {elapsed:.2}s");
}

#[test]
fn backoff_is_clipped_by_remaining_budget() {
    // Python sleeps min(backoff, remaining). With a 3s budget the 5s second
    // backoff must not be slept in full.
    let s = stub_server::start(vec![Some(502), Some(502), Some(200)], 0);
    let mut o = opts(&s.base_url);
    o.deadline_s = Some(3.0);
    let t0 = std::time::Instant::now();
    let _ = send("chat", "hi", &o);
    let elapsed = t0.elapsed().as_secs_f64();
    // Both halves matter. Without the lower bound and the attempt count this
    // passes vacuously for a client that never retries and never sleeps at all.
    assert!(s.attempts() >= 2, "must have retried at least once, got {}", s.attempts());
    assert!(elapsed >= 2.0, "the first 2s backoff must still be slept: {elapsed:.2}s");
    assert!(elapsed < 6.0, "slept past the 3s budget: {elapsed:.2}s");
}

#[test]
fn no_token_returns_false_without_attempting() {
    let s = stub_server::start(vec![Some(200)], 0);
    let o = SendOptions {
        config_path: Some(PathBuf::from("/nonexistent/none.json")),
        base_url: Some(s.base_url.clone()),
        ..Default::default()
    };
    assert!(!send("chat", "hi", &o));
    assert_eq!(s.attempts(), 0);
}
