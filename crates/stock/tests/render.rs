//! Tests written before the implementation.
//!
//! The Python this replaces (`stock/scripts/run.py`, 184 lines, no tests) is
//! evidence of what the skill does today, not a specification. Where it made a
//! decision, that decision is pinned here. Where it merely leaked an
//! implementation detail into the output, this file pins the corrected
//! behaviour and says so.
//!
//! Payloads are real, captured from the live APIs on 2026-08-01.

use stock::quote::{Quote, Source};
use stock::render::{change_suffix, line};
use stock::sources::{parse_twse, parse_yahoo};

// ── change_suffix: the signed delta and percentage ───────────────────────────

#[test]
fn a_rise_is_signed_with_a_plus_on_both_numbers() {
    assert_eq!(
        change_suffix(43119.75, 39933.30).as_deref(),
        Some("+3186.45 (+7.98%)")
    );
}

#[test]
fn a_fall_carries_its_own_minus_and_no_extra_sign() {
    // The Python sets `sign = "+" if change >= 0 else ""` and lets the negative
    // numbers print their own minus. Pinned because a reader of the code might
    // "fix" it into a doubled sign.
    //
    // Note -7.39, not -7.98: the same absolute move measured against the larger
    // base. Writing this test by negating the rise's figure is the mistake it
    // now guards against.
    assert_eq!(
        change_suffix(39933.30, 43119.75).as_deref(),
        Some("-3186.45 (-7.39%)")
    );
}

#[test]
fn an_unchanged_price_is_a_signed_zero() {
    // change >= 0, so the plus applies at exactly zero.
    assert_eq!(change_suffix(100.0, 100.0).as_deref(), Some("+0.00 (+0.00%)"));
}

#[test]
fn a_zero_previous_close_yields_no_suffix_rather_than_infinity() {
    // Python catches ZeroDivisionError and drops the suffix entirely. Rust
    // would produce `inf%` here, which is worse than saying nothing.
    assert_eq!(change_suffix(100.0, 0.0), None);
}

#[test]
fn rounding_matches_python_to_two_places() {
    // Verified against Python's `.2f` on the same values: both round the
    // underlying IEEE754 double half-to-even, so 2.675 renders 2.67 in each.
    // No adjustment is needed here — recorded so nobody adds one.
    assert_eq!(change_suffix(102.675, 100.0).as_deref(), Some("+2.67 (+2.67%)"));
}

// ── line: one rendered quote ─────────────────────────────────────────────────

fn taiex() -> Quote {
    Quote {
        name: "發行量加權股價指數".into(),
        price: "43119.75".into(),
        prev: Some(39933.30),
        price_num: Some(43119.75),
        high: Some("43214.36".into()),
        low: Some("41610.41".into()),
        stamp: Some("2026-07-31 13:33:00".into()),
        source: Source::Twse,
    }
}

#[test]
fn a_full_quote_renders_headline_then_an_indented_detail_line() {
    assert_eq!(
        line(&taiex()),
        "📈 發行量加權股價指數：43119.75 +3186.45 (+7.98%)\n   高 43214.36 / 低 41610.41，2026-07-31 13:33:00"
    );
}

#[test]
fn the_date_is_rendered_with_separators() {
    // Deliberately NOT what the Python did. TWSE returns `d` as "20260731" and
    // the Python interpolated it raw, so the message read "，20260731 13:33:00".
    // That is the wire format leaking into a line a person reads; nobody chose
    // it. Parsed and re-rendered as 2026-07-31.
    assert!(line(&taiex()).contains("2026-07-31 13:33:00"));
    assert!(!line(&taiex()).contains("20260731"));
}

#[test]
fn a_quote_without_high_and_low_is_a_single_line() {
    let q = Quote {
        high: None,
        low: None,
        stamp: None,
        ..taiex()
    };
    assert_eq!(line(&q), "📈 發行量加權股價指數：43119.75 +3186.45 (+7.98%)");
}

#[test]
fn an_unparseable_price_still_renders_the_headline() {
    // TWSE sends "-" for `z` when there has been no trade. The Python's float()
    // raised, the suffix was dropped, and the price printed as "-". Keep that:
    // "no trade yet" is information, and inventing a number would not be.
    let q = Quote {
        price: "-".into(),
        price_num: None,
        ..taiex()
    };
    assert_eq!(
        line(&q),
        "📈 發行量加權股價指數：-\n   高 43214.36 / 低 41610.41，2026-07-31 13:33:00"
    );
}

// ── sources::parse_twse ──────────────────────────────────────────────────────

/// The real TWSE index payload, captured 2026-08-01.
const TWSE_INDEX: &str = r#"{"msgArray":[{"@":"t00.tw","d":"20260731","h":"43214.36","l":"41610.41","n":"發行量加權股價指數","o":"41610.41","t":"13:33:00","y":"39933.30","z":"43119.75"}],"rtcode":"0000"}"#;

#[test]
fn the_twse_payload_maps_onto_a_quote() {
    let q = parse_twse(&serde_json::from_str(TWSE_INDEX).unwrap()).unwrap();
    assert_eq!(q.name, "發行量加權股價指數");
    assert_eq!(q.price, "43119.75");
    assert_eq!(q.prev, Some(39933.30));
    assert_eq!(q.high.as_deref(), Some("43214.36"));
    assert_eq!(q.low.as_deref(), Some("41610.41"));
    assert_eq!(q.stamp.as_deref(), Some("2026-07-31 13:33:00"));
}

#[test]
fn an_empty_msg_array_is_an_error_not_an_empty_quote() {
    // TWSE answers 200 with `msgArray: []` for an unknown symbol. Rendering a
    // blank quote would look like a working lookup of nothing.
    let payload = serde_json::json!({"msgArray": [], "rtcode": "0000"});
    assert!(parse_twse(&payload).is_err());
}

#[test]
fn an_absent_msg_array_is_the_same_error() {
    let payload = serde_json::json!({"rtcode": "5001"});
    assert!(parse_twse(&payload).is_err());
}

// ── sources::parse_yahoo ─────────────────────────────────────────────────────

/// The real Yahoo ^HSI meta, captured 2026-08-01. Note `previousClose` is
/// absent — the live payload carries only `chartPreviousClose`, so the fallback
/// between them is an exercised branch, not a defensive guess.
const YAHOO_HSI: &str = r#"{"chart":{"result":[{"meta":{"regularMarketPrice":25884.43,"chartPreviousClose":24963.23,"regularMarketDayHigh":25917.2,"regularMarketDayLow":25622.92,"regularMarketTime":1785485344,"exchangeTimezoneName":"Asia/Hong_Kong"}}]}}"#;

#[test]
fn the_yahoo_payload_maps_onto_a_quote() {
    let q = parse_yahoo(&serde_json::from_str(YAHOO_HSI).unwrap(), "恒生指數").unwrap();
    assert_eq!(q.name, "恒生指數");
    assert_eq!(q.price, "25884.43");
    assert_eq!(q.prev, Some(24963.23));
}

#[test]
fn yahoo_supplies_the_day_range_the_python_dropped() {
    // The Python's Yahoo path rendered a bare headline while the TWSE path
    // carried 高/低. That asymmetry was not a decision — the HKEX path it
    // replaced had the fields and the replacement simply did not read them.
    // Yahoo does supply them.
    let q = parse_yahoo(&serde_json::from_str(YAHOO_HSI).unwrap(), "恒生指數").unwrap();
    assert_eq!(q.high.as_deref(), Some("25917.2"));
    assert_eq!(q.low.as_deref(), Some("25622.92"));
}

#[test]
fn the_yahoo_timestamp_is_rendered_in_the_exchanges_timezone() {
    // 1785485344 in Asia/Hong_Kong is 2026-07-31 16:09:04, computed
    // independently rather than eyeballed. Rendering it in UTC, or in whatever
    // the host is set to, would put a Hong Kong quote on the wrong clock — and
    // at 16:09 local it would land on the wrong DAY in several timezones.
    let q = parse_yahoo(&serde_json::from_str(YAHOO_HSI).unwrap(), "恒生指數").unwrap();
    assert_eq!(q.stamp.as_deref(), Some("2026-07-31 16:09:04"));
}

#[test]
fn previous_close_wins_over_chart_previous_close_when_present() {
    let payload = serde_json::json!({"chart":{"result":[{"meta":{
        "regularMarketPrice": 100.0,
        "previousClose": 90.0,
        "chartPreviousClose": 80.0
    }}]}});
    assert_eq!(parse_yahoo(&payload, "x").unwrap().prev, Some(90.0));
}

#[test]
fn a_null_previous_close_falls_through_to_the_chart_value() {
    // Exactly the live shape. Python's `or` treats null and 0 alike; this keeps
    // that, because a zero previous close is not a usable divisor either.
    let payload = serde_json::json!({"chart":{"result":[{"meta":{
        "regularMarketPrice": 100.0,
        "previousClose": serde_json::Value::Null,
        "chartPreviousClose": 80.0
    }}]}});
    assert_eq!(parse_yahoo(&payload, "x").unwrap().prev, Some(80.0));
}

#[test]
fn an_empty_result_list_is_an_error() {
    let payload = serde_json::json!({"chart": {"result": []}});
    assert!(parse_yahoo(&payload, "x").is_err());
}
