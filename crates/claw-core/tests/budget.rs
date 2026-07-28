use claw_core::budget::{monotonic_secs, resolve_delivery_deadline, SKILL_STARTED_ENV, SKILL_TIMEOUT_ENV};

/// Process-global env mutation — see the note in tests/marker.rs. Every test in
/// this file takes the lock so the suite is sound even under a parallel harness.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn env_guard() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn clear() {
    std::env::remove_var(SKILL_TIMEOUT_ENV);
    std::env::remove_var(SKILL_STARTED_ENV);
}

#[test]
fn unset_timeout_is_none() {
    let _g = env_guard();
    clear();
    assert_eq!(resolve_delivery_deadline(), None);
}

#[test]
fn malformed_timeout_is_none() {
    let _g = env_guard();
    clear();
    std::env::set_var(SKILL_TIMEOUT_ENV, "not-a-number");
    assert_eq!(resolve_delivery_deadline(), None);
    clear();
}

#[test]
fn non_positive_timeout_is_none() {
    let _g = env_guard();
    clear();
    std::env::set_var(SKILL_TIMEOUT_ENV, "0");
    assert_eq!(resolve_delivery_deadline(), None);
    std::env::set_var(SKILL_TIMEOUT_ENV, "-5");
    assert_eq!(resolve_delivery_deadline(), None);
    clear();
}

#[test]
fn timeout_without_started_reserves_one_second() {
    let _g = env_guard();
    clear();
    std::env::set_var(SKILL_TIMEOUT_ENV, "30");
    assert_eq!(resolve_delivery_deadline(), Some(29.0));
    clear();
}

#[test]
fn malformed_started_falls_back_to_timeout_minus_one() {
    let _g = env_guard();
    clear();
    std::env::set_var(SKILL_TIMEOUT_ENV, "30");
    std::env::set_var(SKILL_STARTED_ENV, "yesterday");
    assert_eq!(resolve_delivery_deadline(), Some(29.0));
    clear();
}

#[test]
fn started_subtracts_elapsed() {
    let _g = env_guard();
    clear();
    std::env::set_var(SKILL_TIMEOUT_ENV, "30");
    // Pretend the skill started 10 monotonic seconds ago.
    std::env::set_var(SKILL_STARTED_ENV, format!("{}", monotonic_secs() - 10.0));
    let got = resolve_delivery_deadline().unwrap();
    assert!((got - 19.0).abs() < 0.5, "expected ~19.0, got {got}");
    clear();
}

#[test]
fn future_start_clamps_elapsed_to_zero() {
    let _g = env_guard();
    clear();
    std::env::set_var(SKILL_TIMEOUT_ENV, "30");
    std::env::set_var(SKILL_STARTED_ENV, format!("{}", monotonic_secs() + 1000.0));
    let got = resolve_delivery_deadline().unwrap();
    assert!((got - 29.0).abs() < 0.5, "expected ~29.0, got {got}");
    clear();
}

#[test]
fn exhausted_budget_floors_at_zero() {
    let _g = env_guard();
    clear();
    std::env::set_var(SKILL_TIMEOUT_ENV, "5");
    std::env::set_var(SKILL_STARTED_ENV, format!("{}", monotonic_secs() - 999.0));
    assert_eq!(resolve_delivery_deadline(), Some(0.0));
    clear();
}

#[test]
fn monotonic_is_not_a_unix_epoch_timestamp() {
    // Regression guard for the clock-domain hazard. A Unix epoch value in 2026
    // is ~1.78e9; CLOCK_MONOTONIC is seconds since boot and will be far smaller
    // on any machine that has not been up for 50 years. If this ever fails,
    // monotonic_secs() has been changed to wall clock and every elapsed
    // computation is silently wrong.
    let t = monotonic_secs();
    assert!(t > 0.0);
    assert!(t < 1.0e9, "monotonic_secs() returned {t}, which looks like wall clock");
}
