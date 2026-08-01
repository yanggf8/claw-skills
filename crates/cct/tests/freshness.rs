//! Ported from cct/scripts/test_run.py, which had 29 cases and was the best
//! oracle in this batch.
//!
//! Both fixtures are real. `stale_payload.json` was captured from the live
//! pre-market route on a day it served a 50-day-old snapshot — the incident
//! these tests exist for — and `testdata/eod-2026-07-29.json` is a genuine
//! scorecard.

use cct::content::{has_eod_data, has_pre_market_data};
use cct::freshness::pre_market_freshness;
use cct::render::format_pre_market;
use jiff::civil::Date;

/// The day the stale payload was captured. Its `date` is 2026-06-08, 50 days
/// earlier.
fn capture_day() -> Date {
    "2026-07-28".parse().unwrap()
}

fn stale() -> serde_json::Value {
    serde_json::from_str(include_str!("stale_payload.json")).expect("fixture parses")
}

/// The same payload as it would look on a healthy day.
fn fresh() -> serde_json::Value {
    let mut v = stale();
    v["date"] = serde_json::json!(capture_day().strftime("%Y-%m-%d").to_string());
    v["is_stale"] = serde_json::json!(false);
    v
}

fn with(mut v: serde_json::Value, key: &str, val: serde_json::Value) -> serde_json::Value {
    if val.is_null() {
        v.as_object_mut().unwrap().remove(key);
    } else {
        v[key] = val;
    }
    v
}

// ── pre_market_freshness ─────────────────────────────────────────────────────

#[test]
fn a_fresh_payload_is_not_stale() {
    let f = pre_market_freshness(&fresh(), capture_day());
    assert!(!f.is_stale);
    assert_eq!(f.age_days, None);
}

#[test]
fn the_real_stale_payload_is_stale_with_its_age() {
    let f = pre_market_freshness(&stale(), capture_day());
    assert!(f.is_stale);
    assert_eq!(f.source_date.as_deref(), Some("2026-06-08"));
    assert_eq!(f.age_days, Some(50));
}

#[test]
fn an_old_date_is_stale_even_when_the_server_omits_the_flag() {
    // Rule 3 earning its place. The server sets is_stale on only two code
    // paths; a third that forgets would otherwise reopen the bug in silence.
    let v = with(stale(), "is_stale", serde_json::Value::Null);
    assert!(pre_market_freshness(&v, capture_day()).is_stale);
}

#[test]
fn the_server_flag_wins_even_when_the_date_is_today() {
    let v = with(fresh(), "is_stale", serde_json::json!(true));
    let f = pre_market_freshness(&v, capture_day());
    assert!(f.is_stale);
    // Today's date, so no positive age to report.
    assert_eq!(f.age_days, None);
}

#[test]
fn a_missing_date_fails_closed() {
    // Freshness cannot be proven, so it is not assumed.
    let v = with(stale(), "date", serde_json::Value::Null);
    let f = pre_market_freshness(&v, capture_day());
    assert!(f.is_stale);
    assert_eq!(f.source_date, None);
}

#[test]
fn an_unparseable_date_fails_closed() {
    let v = with(stale(), "date", serde_json::json!("not-a-date"));
    assert!(pre_market_freshness(&v, capture_day()).is_stale);
}

#[test]
fn a_future_date_is_stale_without_a_negative_age() {
    // "3 天前" for a date in the future would be nonsense; the report says
    // stale and omits the count.
    let v = with(fresh(), "date", serde_json::json!("2026-07-31"));
    let f = pre_market_freshness(&v, capture_day());
    assert!(f.is_stale);
    assert_eq!(f.age_days, None);
}

// ── has_pre_market_data ──────────────────────────────────────────────────────

#[test]
fn fresh_with_content_is_ok() {
    assert!(has_pre_market_data(&fresh(), capture_day()));
}

#[test]
fn the_real_stale_payload_is_not_ok() {
    // A full set of signals, and still not today's analysis. Content alone
    // proves nothing here.
    assert!(!has_pre_market_data(&stale(), capture_day()));
}

#[test]
fn an_old_date_without_the_server_flag_is_not_ok() {
    let v = with(stale(), "is_stale", serde_json::Value::Null);
    assert!(!has_pre_market_data(&v, capture_day()));
}

#[test]
fn fresh_but_empty_is_not_ok() {
    let mut v = fresh();
    v["high_confidence_signals"] = serde_json::json!([]);
    let v = with(v, "symbols_analyzed", serde_json::Value::Null);
    let v = with(v, "overall_sentiment", serde_json::Value::Null);
    assert!(!has_pre_market_data(&v, capture_day()));
}

// ── format_pre_market ────────────────────────────────────────────────────────

#[test]
fn a_fresh_header_shows_the_source_date_without_a_warning() {
    let body = format_pre_market(&fresh(), capture_day());
    assert!(body.contains("2026-07-28"));
    assert!(!body.contains("過期"));
}

#[test]
fn a_stale_header_shows_the_source_date_and_the_age() {
    let body = format_pre_market(&stale(), capture_day());
    let header = body.lines().next().unwrap();
    assert!(header.contains("2026-06-08"));
    assert!(header.contains("50"));
    assert!(header.contains("過期"));
    // The bug: today's date must not appear anywhere in a stale report.
    assert!(!body.contains("2026-07-28"));
}

#[test]
fn a_stale_report_without_an_age_omits_the_day_count() {
    let v = with(
        with(stale(), "date", serde_json::json!("2026-07-28")),
        "is_stale",
        serde_json::json!(true),
    );
    let header = format_pre_market(&v, capture_day())
        .lines()
        .next()
        .unwrap()
        .to_string();
    assert!(header.contains("過期"));
    assert!(!header.contains("天前"));
}

#[test]
fn a_missing_date_renders_as_unknown() {
    let v = with(
        with(stale(), "date", serde_json::Value::Null),
        "is_stale",
        serde_json::Value::Null,
    );
    let header = format_pre_market(&v, capture_day())
        .lines()
        .next()
        .unwrap()
        .to_string();
    assert!(header.contains("日期不明"));
    assert!(header.contains("過期"));
}

#[test]
fn a_stale_report_withholds_the_high_confidence_signals() {
    // The incident. The report carried "⚠️ 資料已過期（50 天前）" and, four
    // lines down, "AAPL 看漲 🟢 95%". 95% was true of 2026-06-08 and says
    // nothing about the morning someone is reading it.
    let body = format_pre_market(&stale(), capture_day());
    assert!(!body.contains("高信心訊號"));
    assert!(!body.contains("95%"));
    assert!(body.contains("2026-06-08"));
}

#[test]
fn a_stale_report_carries_no_percentage_at_all() {
    // Not merely no signal list. What must not survive is a number a reader
    // could act on, so this looks for the percent sign anywhere.
    assert!(!format_pre_market(&stale(), capture_day()).contains('%'));
}

#[test]
fn a_stale_report_says_why_it_is_short() {
    // Short on purpose, and it has to say so, or it reads as a failure to
    // fetch. This reverses an earlier decision that a stale report should
    // still list its signals.
    let body = format_pre_market(&stale(), capture_day());
    assert!(!body.contains("AAPL"));
    assert!(!body.contains("分析標的"));
    assert!(body.contains("不列出"));
    assert!(body.contains("等待今日盤前分析"));
}

#[test]
fn a_fresh_report_still_shows_its_signals() {
    // The withholding is conditional, not a removal of the feature.
    assert!(format_pre_market(&fresh(), capture_day()).contains("高信心訊號"));
}

// ── has_eod_data ─────────────────────────────────────────────────────────────

// Captured from a live end-of-day run. It lives in this crate rather than
// being read out of the cct repo: that path stopped existing the moment the
// skill moved here, and the workspace build has been broken by it since.
const EOD_SCORECARD: &str = include_str!("eod_scorecard.json");

#[test]
fn a_real_scorecard_counts_as_data() {
    // The live shape: flat camelCase, no daily_summary at all. Testing only
    // daily_summary reported degraded on every genuine report from 2026-07-21.
    let v: serde_json::Value = serde_json::from_str(EOD_SCORECARD).unwrap();
    assert!(has_eod_data(&v));
}

#[test]
fn the_placeholder_does_not_count() {
    // What the route synthesises when it finds no snapshot.
    let v = serde_json::json!({"daily_summary": {"symbols_analyzed": 0}});
    assert!(!has_eod_data(&v));
}

#[test]
fn a_scorecard_without_symbols_analyzed_still_counts() {
    let v = serde_json::json!({"signalBreakdown": {"correct": 3}});
    assert!(has_eod_data(&v));
}

#[test]
fn the_legacy_daily_summary_shape_still_counts() {
    let v = serde_json::json!({"daily_summary": {"symbols_analyzed": 5}});
    assert!(has_eod_data(&v));
}

// ── format_eod ───────────────────────────────────────────────────────────────

use cct::render::{eod_session_date, format_eod};

fn scorecard() -> serde_json::Value {
    serde_json::from_str(EOD_SCORECARD).unwrap()
}

#[test]
fn the_eod_header_uses_the_session_date_not_todays() {
    // Stamping a stale report with today's date is the failure the pre-market
    // gate exists to prevent; the same rule applies here.
    let out = format_eod(&scorecard(), "2099-01-01");
    assert!(!out.contains("2099-01-01"), "got:\n{out}");
}

#[test]
fn the_session_date_falls_through_the_timestamps_the_payload_carries() {
    // The scorecard has no `date` — only the placeholder does.
    assert_eq!(
        eod_session_date(&serde_json::json!({"timestamp": "2026-07-29T20:05:00Z"}), "2099-01-01"),
        "2026-07-29"
    );
    assert_eq!(
        eod_session_date(&serde_json::json!({"marketCloseTime": "2026-07-29T20:00:00Z"}), "2099-01-01"),
        "2026-07-29"
    );
    // Nothing usable: only then does it reach for the clock.
    assert_eq!(eod_session_date(&serde_json::json!({}), "2099-01-01"), "2099-01-01");
}

#[test]
fn a_short_timestamp_string_is_not_mistaken_for_a_date() {
    assert_eq!(eod_session_date(&serde_json::json!({"date": "2026"}), "2099-01-01"), "2099-01-01");
}

#[test]
fn the_scorecard_renders_its_grade_and_hit_rate() {
    let out = format_eod(&scorecard(), "2099-01-01");
    assert!(out.contains("模型評級") || out.contains("高信心命中"), "got:\n{out}");
}

#[test]
fn the_placeholder_still_renders_without_panicking() {
    // This test originally used `market_sentiment`, copied from an
    // implementation that had the key wrong. It passed against the wrong code
    // and would have passed forever — a test written from the implementation
    // rather than from the payload. The live placeholder says
    // `overall_sentiment`.
    let v = serde_json::json!({"daily_summary": {"symbols_analyzed": 0, "overall_sentiment": "neutral"}});
    let out = format_eod(&v, "2026-08-01");
    assert!(out.starts_with("📊 CCT 收盤報告｜2026-08-01"));
    assert!(out.contains("中性"));
}

#[test]
fn a_signal_row_carries_its_arrow_and_outcome() {
    let v = serde_json::json!({
        "signalBreakdown": [
            {"ticker": "AAPL", "predicted": "↑ Expected up", "actual": "↓ 0.6%", "correct": false, "confidence": 82}
        ]
    });
    let out = format_eod(&v, "2026-08-01");
    // The arrow only, and the actual tightened so the row fits a phone.
    assert!(out.contains("預測↑ 實際↓0.6%  ✗ 82%"), "got:\n{out}");
}

#[test]
fn the_placeholder_summary_reads_overall_sentiment_not_market_sentiment() {
    // The live placeholder carries `overall_sentiment`. An earlier version of
    // this port read `market_sentiment` and silently dropped the 今日總結 line
    // from every placeholder report. No unit test caught it — a differential
    // run against the Python did, on the live payload.
    let v = serde_json::json!({
        "daily_summary": {"symbols_analyzed": 0, "overall_sentiment": "neutral",
                          "key_events": ["Market closed", "EOD analysis not yet available"]}
    });
    let out = format_eod(&v, "2026-08-01");
    assert!(out.contains("今日總結：中性 ⚪"), "got:\n{out}");
}

#[test]
fn a_summary_confidence_is_appended_when_present() {
    let v = serde_json::json!({
        "daily_summary": {"overall_sentiment": "bullish", "confidence": 0.82}
    });
    assert!(format_eod(&v, "d").contains("今日總結：看漲 🟢（信心 82%）"));
}

#[test]
fn the_eod_branch_is_chosen_by_shape_not_by_output() {
    // A scorecard thin enough to render nothing still takes the scorecard
    // branch; deciding by "did the renderer produce lines" would fall through
    // to the placeholder path and print a summary the payload never had.
    let v = serde_json::json!({"totalSignals": 0, "daily_summary": {"overall_sentiment": "bullish"}});
    assert!(!format_eod(&v, "d").contains("今日總結"));
}
