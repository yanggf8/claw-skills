//! Whether a pre-market payload describes today's analysis.

use jiff::civil::Date;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Freshness {
    /// `data["date"]` as received, or None when absent or unparseable.
    pub source_date: Option<String>,
    pub is_stale: bool,
    /// Days between source_date and today, only when positive.
    pub age_days: Option<i64>,
}

/// Single source of truth for pre-market staleness.
///
/// The pre-market route falls back to the latest D1 snapshot when today's job
/// never ran, so a payload can carry a full set of signals and still describe a
/// market day weeks in the past. Stale if any of:
///
///   1. the server says so via `is_stale`,
///   2. the date is absent or unparseable — freshness cannot be proven,
///   3. the date is not today.
///
/// Rule 3 is not redundant with rule 1: the server sets `is_stale` on only two
/// code paths, so a future path that forgets the flag would otherwise reopen
/// this silently.
///
/// The comparison is in UTC deliberately, matching the report route, which
/// derives its own `today` as `new Date().toISOString().split('T')[0]`. Jobs are
/// *written* under an ET business date, so writer and reader already disagree
/// upstream; matching only one side would make the skill contradict the very
/// flag it cross-checks.
pub fn pre_market_freshness(data: &serde_json::Value, today: Date) -> Freshness {
    let raw = data.get("date").and_then(|v| v.as_str());
    let parsed: Option<Date> = raw.and_then(|s| s.parse().ok());

    let Some(d) = parsed else {
        return Freshness {
            source_date: None,
            is_stale: true,
            age_days: None,
        };
    };

    let age = (today - d).get_days() as i64;
    Freshness {
        source_date: raw.map(str::to_string),
        is_stale: data
            .get("is_stale")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
            || d != today,
        age_days: if age > 0 { Some(age) } else { None },
    }
}

/// The clock a report's own date must be judged against.
///
/// `business_date` comes off the envelope and is an **ET** business date — ET is
/// the market's own time, so the trading day is the ET date. The legacy
/// `data["date"]` is what the route served before it learned that difference,
/// and is only comparable to **UTC**. One clock cannot judge both: for the four
/// to five hours after 00:00 UTC the UTC day is already tomorrow while the ET
/// session is still today, which is exactly when end-of-day work lands. Judging
/// an ET date by a UTC clock in that window calls a fresh briefing a day old.
///
/// Binding the clock to the field's provenance, rather than switching globally,
/// is also what lets this skill and the worker deploy in either order. A skill
/// that moved to ET unconditionally would break the other direction — calling
/// fresh reports stale for the same hours the original defect covered — for as
/// long as it ran ahead of the worker.
pub fn comparison_today(business_date: Option<&str>, et_today: Date, utc_today: Date) -> Date {
    if business_date.is_some() {
        et_today
    } else {
        utc_today
    }
}
