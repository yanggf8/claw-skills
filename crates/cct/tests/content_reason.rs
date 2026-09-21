//! Why a degraded verdict has to say why, the same way a rejected envelope does.
//!
//! On 2026-08-07 the eod cron reported `contract_degraded` and the alert again
//! carried **no stderr** — nullclaw prints the literal string "no stderr" when
//! the skill wrote none. `unwrap_envelope` was not at fault this time: the
//! payload arrived intact and `has_eod_data` refused it, and that branch of
//! main was silent. Same symptom, other half of the fork.
//!
//! The reason is not decoration. "EOD analysis not yet available" points at the
//! upstream job; "stale: payload date=…" points at a run that happened and
//! produced yesterday's answer; an unreachable host points at the network.
//! Without the line all three read identically at the alert.
//!
//! These tests pin the reason, not just the `Some`.

use cct::cli::Mode;
use cct::content::{
    content_gap, has_eod_data, has_intraday_data, has_pre_market_data, has_weekly_data,
};
use jiff::civil::Date;

/// Captured live from the end-of-day route on 2026-08-07, the morning the
/// upstream EOD job never ran: GitHub Actions dropped the 20:05 UTC schedule
/// entirely after the 16:00 UTC one failed to acquire a runner.
const EOD_PLACEHOLDER: &str = include_str!("eod_placeholder.json");
/// Captured in the same minute from the intraday route.
const INTRADAY_EMPTY: &str = include_str!("intraday_empty.json");
const STALE_PRE_MARKET: &str = include_str!("stale_payload.json");
const EOD_SCORECARD: &str = include_str!("eod_scorecard.json");

fn json(s: &str) -> serde_json::Value {
    serde_json::from_str(s).expect("fixture parses")
}

/// The day the placeholders were captured.
fn today() -> Date {
    "2026-08-07".parse().expect("date")
}

#[test]
fn the_eod_placeholder_gap_quotes_the_route_back() {
    let gap = content_gap(Mode::Eod, &json(EOD_PLACEHOLDER), today()).expect("a gap");
    assert!(
        gap.contains("EOD analysis not yet available"),
        "the route explains itself in key_events; quote it. got: {gap}"
    );
}

#[test]
fn the_intraday_gap_quotes_the_message_the_payload_carries() {
    // The message names the fix — POST /api/v1/jobs/intraday — which is the
    // whole reason to forward it rather than paraphrase.
    let gap = content_gap(Mode::Intraday, &json(INTRADAY_EMPTY), today()).expect("a gap");
    assert!(
        gap.contains("No intraday data available"),
        "got: {gap}"
    );
}

#[test]
fn a_stale_pre_market_gap_names_the_date_it_actually_got() {
    // Stale is the case that reads as healthy at a glance: a full set of
    // signals, every field populated, describing a market day two months back.
    // The date is what separates it from an empty payload.
    let gap = content_gap(Mode::PreMarket, &json(STALE_PRE_MARKET), today()).expect("a gap");
    assert!(gap.contains("2026-06-08"), "got: {gap}");
    assert!(gap.contains("2026-08-07"), "got: {gap}");
}

#[test]
fn a_real_scorecard_leaves_no_gap_and_says_nothing() {
    assert_eq!(content_gap(Mode::Eod, &json(EOD_SCORECARD), today()), None);
}

#[test]
fn a_fresh_pre_market_payload_leaves_no_gap() {
    let fresh = json(
        r#"{"type":"pre_market_briefing","date":"2026-08-07","is_stale":false,
            "high_confidence_signals":[{"symbol":"AAPL"}]}"#,
    );
    assert_eq!(content_gap(Mode::PreMarket, &fresh, today()), None);
}

#[test]
fn a_weekly_payload_without_a_report_names_what_is_missing() {
    let gap = content_gap(Mode::Weekly, &json(r#"{"report":{}}"#), today()).expect("a gap");
    assert!(gap.contains("weekly_overview"), "got: {gap}");
}

#[test]
fn no_mode_can_report_a_gap_without_a_reason() {
    // Silence is the bug this file exists for, so an empty payload — the shape
    // no branch was written for — must still produce words in every mode.
    for mode in [Mode::PreMarket, Mode::Intraday, Mode::Eod, Mode::Weekly] {
        let gap = content_gap(mode, &json("{}"), today());
        let reason = gap.unwrap_or_else(|| panic!("{mode:?} called an empty payload usable"));
        assert!(!reason.trim().is_empty(), "{mode:?} gave a blank reason");
    }
}

#[test]
fn the_gap_agrees_with_the_predicate_it_explains() {
    // The two must not drift: a gap with no degrade would print a reason for a
    // report the user reads as ok, and a degrade with no gap is the 2026-08-07
    // alert all over again.
    let cases = [
        json(EOD_PLACEHOLDER),
        json(INTRADAY_EMPTY),
        json(STALE_PRE_MARKET),
        json(EOD_SCORECARD),
        json("{}"),
        json(r#"{"total_symbols":3,"symbols":[{"symbol":"AAPL"}]}"#),
        json(r#"{"report":{"weekly_overview":{"accuracy":0.6}}}"#),
        json(r#"{"date":"2026-08-07","is_stale":false,"symbols_analyzed":5}"#),
    ];
    for data in &cases {
        for mode in [Mode::PreMarket, Mode::Intraday, Mode::Eod, Mode::Weekly] {
            let has = match mode {
                Mode::PreMarket => has_pre_market_data(data, today()),
                Mode::Intraday => has_intraday_data(data),
                Mode::Eod => has_eod_data(data),
                Mode::Weekly => has_weekly_data(data),
            };
            assert_eq!(
                content_gap(mode, data, today()).is_none(),
                has,
                "{mode:?} disagrees with its predicate on {data}"
            );
        }
    }
}

#[test]
fn the_mode_slug_is_the_spelling_the_operator_typed() {
    // The alert names the skill but not the mode, and four cron jobs share the
    // skill name. The slug has to round-trip with --mode or the warning sends
    // the reader to the wrong job.
    for mode in [Mode::PreMarket, Mode::Intraday, Mode::Eod, Mode::Weekly] {
        assert_eq!(Mode::parse(mode.slug()), Some(mode));
    }
}
