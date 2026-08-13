//! Market time. Every date and hour this skill reports is US Eastern.
//!
//! The reports are named for a *session* — "盤前" is before an ET open, "收盤"
//! is after an ET close — so the session's own clock is the only one that can
//! name them. A UTC date is not a trading day: for the four to five hours after
//! 00:00 UTC the UTC day is already tomorrow while the ET session is still
//! today, which is exactly when end-of-day work lands. See the ET rule in
//! `CLAUDE.md`.
//!
//! Before 2026-08-13 the report header was stamped with a UTC date. It happened
//! to agree with ET at both scheduled times, so nothing was visibly wrong —
//! which is the whole hazard: the agreement was a property of the schedule, and
//! the schedule is exactly what this change moves.

use jiff::{tz::TimeZone, Timestamp, Zoned};

pub const MARKET_TZ: &str = "America/New_York";

/// Now, in market time. `None` when the tz database is unavailable.
pub fn market_now() -> Option<Zoned> {
    TimeZone::get(MARKET_TZ)
        .ok()
        .map(|tz| Timestamp::now().to_zoned(tz))
}

/// The trading day, derived in the market's own zone — never from a UTC date,
/// and never by round-tripping an ET value back through UTC.
pub fn business_date(now: &Zoned) -> String {
    now.strftime("%Y-%m-%d").to_string()
}

/// How the report states when it was made: market time, with the abbreviation
/// that carries DST (`EDT` / `EST`).
///
/// The abbreviation is not decoration. A reader who sees `16:10 EDT` can tell
/// the run happened after the close; `16:10` alone leaves them to guess which
/// offset was in force, and the offset is what moved.
pub fn market_stamp(now: &Zoned) -> String {
    now.strftime("%H:%M %Z").to_string()
}

/// Whether this run is the one the schedule meant.
///
/// The scheduler fires on a fixed UTC expression, so a job pinned to an ET hour
/// must be scheduled at **both** UTC hours the year can put it at and let the
/// wrong one skip. Same shape as doughcon's `--et-hour`, for the same reason.
///
/// `None` means no gate was requested and the run always proceeds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Gate {
    Run,
    Skip { current_hour: i32, abbrev: String },
}

pub fn gate(current_hour: i32, abbrev: &str, target: Option<i32>) -> Gate {
    match target {
        Some(t) if t != current_hour => Gate::Skip {
            current_hour,
            abbrev: abbrev.to_string(),
        },
        _ => Gate::Run,
    }
}
