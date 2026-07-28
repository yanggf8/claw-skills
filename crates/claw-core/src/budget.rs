//! Wall-clock budget for delivery, derived from the cron environment.
//!
//! CLOCK DOMAIN: NULLCLAW_SKILL_STARTED is CLOCK_MONOTONIC seconds, matching
//! Python's time.monotonic() on Linux. A wall-clock producer would make every
//! elapsed computation silently zero (the negative difference clamps), so the
//! producer side (nullclaw) and this consumer must agree. See tests.

pub const SKILL_TIMEOUT_ENV: &str = "NULLCLAW_SKILL_TIMEOUT";
pub const SKILL_STARTED_ENV: &str = "NULLCLAW_SKILL_STARTED";

pub fn monotonic_secs() -> f64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: ts is a valid, properly aligned timespec we own.
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    ts.tv_sec as f64 + ts.tv_nsec as f64 / 1e9
}

/// None means "no budget information" — telegram falls back to its own cap.
/// Every malformed input degrades to None or to the timeout-only path; nothing
/// here fails loudly, matching Python.
pub fn resolve_delivery_deadline() -> Option<f64> {
    let raw_timeout = std::env::var(SKILL_TIMEOUT_ENV).ok().filter(|v| !v.is_empty())?;
    // Python's float() strips surrounding whitespace; f64::from_str does not.
    let timeout: f64 = raw_timeout.trim().parse().ok()?;
    if timeout <= 0.0 {
        return None;
    }

    if let Ok(raw_started) = std::env::var(SKILL_STARTED_ENV) {
        if !raw_started.is_empty() {
            if let Ok(started) = raw_started.trim().parse::<f64>() {
                let elapsed = (monotonic_secs() - started).max(0.0);
                let remaining = (timeout - elapsed).max(0.0);
                // Reserve 1s for the skill to exit cleanly after delivery.
                return Some((remaining - 1.0).max(0.0));
            }
        }
    }
    Some((timeout - 1.0).max(0.0))
}
