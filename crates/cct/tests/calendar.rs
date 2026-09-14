//! The ET trading calendar, against the dates this incident produced.

use cct::calendar::{is_trading_day, prev_trading_day, weekday_short};
use jiff::civil::Date;

fn d(s: &str) -> Date {
    s.parse().unwrap()
}

#[test]
fn the_2026_incident_weekend_is_not_a_trading_day() {
    // 2026-09-12: the catch-up drained Friday's missed slots onto this ET
    // Saturday and wrote a ghost report. The gate exists to refuse it.
    assert!(!is_trading_day(d("2026-09-12")));
    assert!(!is_trading_day(d("2026-09-13")));
    assert!(is_trading_day(d("2026-09-11")));
}

#[test]
fn labor_day_2026_is_a_holiday() {
    assert!(!is_trading_day(d("2026-09-07")));
}

#[test]
fn an_unknown_year_lets_weekdays_through() {
    // 2030-07-04 is a Thursday; the table has no 2030, and the rule is that
    // an unknown year cannot be claimed holiday-free — the callers surface
    // the uncertainty instead of silently skipping a session.
    assert!(is_trading_day(d("2030-07-04")));
}

#[test]
fn prev_trading_day_steps_over_the_weekend() {
    assert_eq!(prev_trading_day(d("2026-09-12")), d("2026-09-11"));
    assert_eq!(prev_trading_day(d("2026-09-14")), d("2026-09-11"));
    assert_eq!(prev_trading_day(d("2026-09-15")), d("2026-09-14"));
}

#[test]
fn prev_trading_day_steps_over_a_holiday() {
    // Tuesday 2026-09-08: Monday was Labor Day, so the previous session is
    // Friday 2026-09-04.
    assert_eq!(prev_trading_day(d("2026-09-08")), d("2026-09-04"));
}

#[test]
fn weekdays_abbreviate_like_the_alert_did() {
    assert_eq!(weekday_short(d("2026-09-12").weekday()), "Sat");
    assert_eq!(weekday_short(d("2026-09-14").weekday()), "Mon");
}
