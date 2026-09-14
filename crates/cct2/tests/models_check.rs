//! The models-journal verdict, against the shapes the 09-10 journal row and
//! the 09-11 outage produced.

use cct2::models_check::{evaluate, last_expected_weekday};
use jiff::civil::Date;
use serde_json::json;

fn row(mode: &str, date: &str, answered: bool, tickers: i64) -> serde_json::Value {
    json!({
        "mode": mode,
        "business_date": date,
        "primary": {"answered": answered, "model": "MiniMax-M3", "tickers": tickers},
        "backup": {"answered": true, "model": "GLM-5.1", "tickers": tickers},
        "requested": 5
    })
}

fn healthy_day(date: &str) -> Vec<serde_json::Value> {
    vec![row("pre-market", date, true, 5), row("eod", date, true, 5)]
}

#[test]
fn both_modes_answered_is_ok() {
    let v = evaluate(&healthy_day("2026-09-10"), Some("2026-09-10"));
    assert!(v.ok, "{v:?}");
    assert!(v.reasons.is_empty());
}

#[test]
fn a_quiet_primary_is_a_finding_even_when_the_backup_answers() {
    // 2026-09-10, for real: MiniMax burned its budget on thinking, GLM
    // answered, the report shipped with a 單一模型回應 footer. This check
    // exists to flag exactly that.
    let mut entries = healthy_day("2026-09-10");
    entries[0] = row("pre-market", "2026-09-10", false, 0);
    let v = evaluate(&entries, Some("2026-09-10"));
    assert!(!v.ok);
    assert!(v.reasons.iter().any(|r| r.contains("pre-market primary not answered")), "{v:?}");
}

#[test]
fn a_short_ticker_count_is_a_finding() {
    let mut entries = healthy_day("2026-09-10");
    entries[0] = row("pre-market", "2026-09-10", true, 3);
    let v = evaluate(&entries, Some("2026-09-10"));
    assert!(!v.ok);
    assert!(
        v.reasons.iter().any(|r| r.contains("primary tickers 3 < requested 5")),
        "{v:?}"
    );
}

#[test]
fn a_missing_mode_is_a_finding() {
    let entries = vec![row("pre-market", "2026-09-10", true, 5)];
    let v = evaluate(&entries, Some("2026-09-10"));
    assert!(!v.ok);
    assert!(v.reasons.iter().any(|r| r.contains("eod missing for 2026-09-10")), "{v:?}");
}

#[test]
fn a_journal_that_stops_short_names_itself_stale() {
    // The 09-11..12 shape: 09-10's primary miss keeps being the verdict
    // because nothing newer landed. The repeat must say "stale", not dress
    // the old finding up as a fresh one.
    let mut entries = healthy_day("2026-09-10");
    entries[0] = row("pre-market", "2026-09-10", false, 0);
    let v = evaluate(&entries, Some("2026-09-11"));
    assert!(!v.ok);
    assert!(
        v.reasons
            .iter()
            .any(|r| r.contains("journal is stale: last entry 2026-09-10, expected 2026-09-11")),
        "{v:?}"
    );
}

#[test]
fn weekends_look_back_to_friday() {
    // The check runs at 00:30Z: on an ET Saturday or Sunday evening the
    // journal should reach Friday. Monday expects Monday itself — the
    // current day counts.
    let sat: Date = "2026-09-12".parse().unwrap();
    let sun: Date = "2026-09-13".parse().unwrap();
    let mon: Date = "2026-09-14".parse().unwrap();
    let fri: Date = "2026-09-11".parse().unwrap();
    assert_eq!(last_expected_weekday(sat), fri);
    assert_eq!(last_expected_weekday(sun), fri);
    assert_eq!(last_expected_weekday(mon), mon);
}
