// `#[allow(dead_code)]`: the stub server is shared with tests/telegram.rs, which
// uses `Stub::body`. This binary does not, and each test binary is compiled
// separately, so without the allow the shared helper warns here. Scoped to the
// module declaration so nothing else in this file is silenced.
mod support { #[allow(dead_code)] pub mod stub_server; }
use claw_core::delivery::{deliver, DeliverOptions, DeliveryOutcome};
use std::io::Write;
use std::path::PathBuf;
use support::stub_server;

static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn cfg() -> PathBuf {
    let mut p = std::env::temp_dir();
    // One file per CALL, not per process. Sharing one path meant File::create
    // truncated it while another thread's send() was reading, yielding an empty
    // file -> no token -> a silent zero-attempt "failure" in an unrelated test.
    static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    p.push(format!("claw-core-del-{}-{}.json", std::process::id(), n));
    let mut f = std::fs::File::create(&p).unwrap();
    f.write_all(br#"{"channels":{"telegram":{"botToken":"T"}}}"#).unwrap();
    p
}

fn opts(base: &str) -> DeliverOptions {
    DeliverOptions { config_path: Some(cfg()), base_url: Some(base.into()), ..Default::default() }
}

fn run(chat: Option<&str>, body: &str, o: &DeliverOptions) -> (DeliveryOutcome, String, String) {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let r = deliver(chat, body, o, &mut out, &mut err);
    (r, String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap())
}

fn env_guard() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

#[test]
fn none_chat_prints_body_to_stdout() {
    let _g = env_guard();
    let s = stub_server::start(vec![], 0);
    let (r, out, err) = run(None, "hello", &opts(&s.base_url));
    assert!(matches!(r, DeliveryOutcome::PrintedToStdout));
    assert_eq!(out, "hello\n");
    assert_eq!(err, "");
    assert_eq!(s.attempts(), 0);
}

#[test]
fn empty_chat_prints_body_to_stdout() {
    let _g = env_guard();
    let s = stub_server::start(vec![], 0);
    let (r, out, _) = run(Some(""), "hello", &opts(&s.base_url));
    assert!(matches!(r, DeliveryOutcome::PrintedToStdout));
    assert_eq!(out, "hello\n");
    assert_eq!(s.attempts(), 0);
}

#[test]
fn success_emits_nothing() {
    let _g = env_guard();
    let s = stub_server::start(vec![Some(200)], 0);
    let (r, out, err) = run(Some("chat"), "hello", &opts(&s.base_url));
    assert!(matches!(r, DeliveryOutcome::Sent));
    assert_eq!(out, "", "channel has the body; do not echo it");
    assert_eq!(err, "");
}

#[test]
fn failure_default_is_fatal_and_preserves_body_on_stdout() {
    let _g = env_guard();
    let s = stub_server::start(vec![Some(403)], 0);
    let (r, out, err) = run(Some("chat9"), "hello", &opts(&s.base_url));
    assert!(matches!(r, DeliveryOutcome::FailedFatal));
    assert_eq!(out, "hello\n", "body must survive on stdout for cron capture");
    assert!(err.contains("[delivery] telegram send failed for chat=chat9 account=main"));
}

#[test]
fn env_budget_is_actually_passed_through_to_telegram() {
    // budget and telegram are unit-tested separately; without this the JOIN is
    // untested and a `deadline_s: None` regression would pass every other test.
    // A 1s skill timeout leaves ~0s of delivery budget, so send must abandon
    // before its first HTTP attempt against a hanging stub.
    let _g = env_guard();
    let s = stub_server::start(vec![None], 5000);
    std::env::set_var("NULLCLAW_SKILL_TIMEOUT", "1");
    std::env::remove_var("NULLCLAW_SKILL_STARTED");
    let t0 = std::time::Instant::now();
    let (r, out, _err) = run(Some("chat"), "hello", &opts(&s.base_url));
    std::env::remove_var("NULLCLAW_SKILL_TIMEOUT");
    assert!(matches!(r, DeliveryOutcome::FailedFatal));
    assert_eq!(out, "hello\n", "body still preserved on stdout");
    assert!(
        t0.elapsed().as_secs_f64() < 4.0,
        "budget was not applied — delivery ran past the 1s skill timeout"
    );
}

#[test]
fn failure_opt_out_is_soft_but_still_writes_both() {
    let _g = env_guard();
    let s = stub_server::start(vec![Some(403)], 0);
    let mut o = opts(&s.base_url);
    o.fail_on_delivery_error = false;
    let (r, out, err) = run(Some("chat9"), "hello", &o);
    assert!(matches!(r, DeliveryOutcome::FailedSoft));
    assert_eq!(out, "hello\n");
    assert!(err.contains("[delivery] telegram send failed"));
}

#[test]
fn parse_mode_actually_reaches_the_request() {
    // Deleting `parse_mode: opts.parse_mode.clone()` from deliver() previously
    // passed every test while silently removing Markdown from every message.
    let _g = env_guard();
    let s = stub_server::start(vec![Some(200), Some(200)], 0);
    let mut o = opts(&s.base_url);
    let _ = run(Some("chat"), "body", &o);
    o.parse_mode = None;
    let _ = run(Some("chat"), "body", &o);
    assert!(s.body(0).contains("\"parse_mode\":\"Markdown\""), "default must survive deliver()");
    assert!(!s.body(1).contains("parse_mode"), "None must omit the key entirely");
}
