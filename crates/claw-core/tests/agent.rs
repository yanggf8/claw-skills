use claw_core::agent::{agent_binary_path, call_agent};
use std::io::Write;
use std::time::Duration;

static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
fn guard() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// Plant a fake agent under a temporary HOME. This is exactly the seam the
/// differential harness uses, so exercising it here keeps it honest.
fn fake_home(script: &str) -> std::path::PathBuf {
    static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut home = std::env::temp_dir();
    home.push(format!("claw-core-home-{}-{}", std::process::id(), n));
    let bin = home.join("nullclaw/zig-out/bin");
    std::fs::create_dir_all(&bin).unwrap();
    let p = bin.join("nullclaw");
    std::fs::File::create(&p).unwrap().write_all(script.as_bytes()).unwrap();
    std::fs::set_permissions(&p, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    home
}

#[test]
fn resolves_the_binary_through_home() {
    let _g = guard();
    std::env::set_var("HOME", "/tmp/fake-home-probe");
    assert_eq!(
        agent_binary_path(),
        std::path::PathBuf::from("/tmp/fake-home-probe/nullclaw/zig-out/bin/nullclaw")
    );
}

#[test]
fn returns_sanitized_stdout() {
    let _g = guard();
    let home = fake_home("#!/bin/sh\nprintf 'take an umbrella<ncchoices>junk'\n");
    std::env::set_var("HOME", &home);
    assert_eq!(call_agent("p", Duration::from_secs(5)), "take an umbrella");
}

#[test]
fn ignores_a_nonzero_exit_code() {
    // B13. Python never checks returncode — stdout is used regardless. A Rust
    // port that treats exit != 0 as failure silently drops real advice.
    let _g = guard();
    let home = fake_home("#!/bin/sh\nprintf 'advice anyway'\nexit 3\n");
    std::env::set_var("HOME", &home);
    assert_eq!(call_agent("p", Duration::from_secs(5)), "advice anyway");
}

#[test]
fn empty_stdout_yields_empty_advice() {
    let _g = guard();
    let home = fake_home("#!/bin/sh\nexit 0\n");
    std::env::set_var("HOME", &home);
    assert_eq!(call_agent("p", Duration::from_secs(5)), "");
}

#[test]
fn missing_binary_yields_empty_advice_not_a_panic() {
    let _g = guard();
    std::env::set_var("HOME", "/nonexistent/claw-core-home");
    assert_eq!(call_agent("p", Duration::from_secs(5)), "");
}

#[test]
fn timeout_yields_empty_advice() {
    let _g = guard();
    let home = fake_home("#!/bin/sh\nsleep 30\n");
    std::env::set_var("HOME", &home);
    let t0 = std::time::Instant::now();
    assert_eq!(call_agent("p", Duration::from_millis(500)), "");
    assert!(t0.elapsed().as_secs() < 5, "must not wait for the child");
}

#[test]
fn prompt_reaches_the_child_as_a_single_argv_entry() {
    let _g = guard();
    let home = fake_home("#!/bin/sh\nprintf '%s' \"$3\"\n");
    std::env::set_var("HOME", &home);
    // argv is ["agent", "-m", prompt] => $3 is the prompt, intact with spaces.
    assert_eq!(call_agent("a b  c", Duration::from_secs(5)), "a b  c");
}
