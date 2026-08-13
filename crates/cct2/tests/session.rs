//! Market time, the prediction journal, and the close-vs-morning review.

use cct2::clock::{business_date, gate, market_stamp, Gate, MARKET_TZ};
use cct2::journal::{from_json, journal_path, load, save, to_json, Journal, Prediction};
use cct2::merge::{Agreement, Opinion, Row};
use cct2::render::{format_report, format_review, ReportContext};
use cct2::review::{pct_change, review, score, tally, Outcome, Reviewed};
use jiff::{tz::TimeZone, Timestamp};
use std::sync::atomic::{AtomicUsize, Ordering};

fn et(instant: &str) -> jiff::Zoned {
    instant
        .parse::<Timestamp>()
        .expect("literal timestamp")
        .to_zoned(TimeZone::get(MARKET_TZ).expect("bundled tzdb"))
}

// ── the trading day is ET, never UTC ─────────────────────────────────────────

#[test]
fn the_business_date_is_the_et_day_even_when_utc_has_rolled_over() {
    // 01:30 UTC on the 13th is 21:30 ET on the 12th. A UTC-derived date names
    // the 13th — a day the session in question never touched. This is the
    // window every end-of-day job lands in.
    let z = et("2026-08-13T01:30:00Z");
    assert_eq!(business_date(&z), "2026-08-12");
    assert_eq!(market_stamp(&z), "21:30 EDT");
}

#[test]
fn the_close_run_is_dated_to_the_session_that_just_ended() {
    // 20:10 UTC = 16:10 EDT, forty minutes after the close.
    let z = et("2026-08-12T20:10:00Z");
    assert_eq!(business_date(&z), "2026-08-12");
    assert_eq!(market_stamp(&z), "16:10 EDT");
}

#[test]
fn the_stamp_carries_the_dst_abbreviation_that_is_actually_in_force() {
    // Same wall-clock hour, opposite sides of the DST boundary. Without the
    // abbreviation these two are indistinguishable in the report, and they are
    // an hour apart in UTC — which is exactly what the schedule has to absorb.
    assert_eq!(market_stamp(&et("2026-08-12T12:30:00Z")), "08:30 EDT");
    assert_eq!(market_stamp(&et("2026-12-10T13:30:00Z")), "08:30 EST");
}

// ── the DST gate ─────────────────────────────────────────────────────────────

#[test]
fn the_gate_runs_only_on_the_hour_the_schedule_meant() {
    assert_eq!(gate(8, "EDT", Some(8)), Gate::Run);
    assert_eq!(
        gate(9, "EDT", Some(8)),
        Gate::Skip {
            current_hour: 9,
            abbrev: "EDT".to_string()
        }
    );
}

#[test]
fn no_target_means_the_gate_was_not_requested_and_never_skips() {
    // A manual run passes no `--et-hour` and must not be silently skipped.
    assert_eq!(gate(3, "EST", None), Gate::Run);
    assert_eq!(gate(23, "EDT", None), Gate::Run);
}

#[test]
fn the_pre_market_pair_fires_exactly_once_across_the_dst_boundary() {
    // The job is scheduled at both 12:30 and 13:30 UTC with `--et-hour 8`.
    // Exactly one of them may run, in either half of the year — that is the
    // whole contract, and it is what a fixed-offset cron cannot express.
    let hour_at = |instant: &str| et(instant).hour() as i32;

    // 12:30 UTC — the summer slot.
    assert_eq!(gate(hour_at("2026-08-12T12:30:00Z"), "EDT", Some(8)), Gate::Run);
    assert!(matches!(
        gate(hour_at("2026-12-10T12:30:00Z"), "EST", Some(8)),
        Gate::Skip { .. }
    ));

    // 13:30 UTC — the winter slot, and the two never both run.
    assert!(matches!(
        gate(hour_at("2026-08-12T13:30:00Z"), "EDT", Some(8)),
        Gate::Skip { .. }
    ));
    assert_eq!(gate(hour_at("2026-12-10T13:30:00Z"), "EST", Some(8)), Gate::Run);
}

// ── the journal ──────────────────────────────────────────────────────────────

static SEQ: AtomicUsize = AtomicUsize::new(0);

/// One directory per call. Cargo runs these on several threads in one binary,
/// and a shared path lets one test read another's truncation window.
fn tmp_home() -> std::path::PathBuf {
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("cct2-journal-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&p).expect("temp home");
    p
}

fn sample() -> Journal {
    Journal {
        business_date: "2026-08-12".into(),
        made_at: "08:30 EDT".into(),
        predictions: vec![
            Prediction {
                ticker: "AAPL".into(),
                sentiment: "bullish".into(),
                confidence: 0.82,
                reference_price: Some(200.0),
            },
            Prediction {
                ticker: "TSLA".into(),
                sentiment: "bearish".into(),
                confidence: 0.6,
                reference_price: None,
            },
        ],
    }
}

#[test]
fn a_journal_survives_a_write_and_a_read() {
    let home = tmp_home();
    let j = sample();
    let path = save(&home, &j).expect("save");
    assert_eq!(path, journal_path(&home, "2026-08-12"));
    assert_eq!(load(&home, "2026-08-12").as_ref(), Some(&j));
}

#[test]
fn a_missing_price_round_trips_as_missing_rather_than_zero() {
    // Stored as zero it would score as a flat day against a real close, which
    // is a fabricated reading rather than an absent one.
    let back = from_json(&to_json(&sample())).expect("parse");
    assert_eq!(back.predictions[1].reference_price, None);
}

#[test]
fn a_journal_for_another_day_is_not_returned() {
    let home = tmp_home();
    save(&home, &sample()).expect("save");
    assert!(load(&home, "2026-08-11").is_none());
}

#[test]
fn an_unreadable_journal_is_the_same_as_no_journal() {
    let home = tmp_home();
    let dir = home.join(".nullclaw/skills/cct2/journal");
    std::fs::create_dir_all(&dir).expect("dir");
    std::fs::write(dir.join("2026-08-12.json"), "{ this is not json").expect("write");
    assert!(load(&home, "2026-08-12").is_none());

    // Well-formed JSON that is not a journal is equally unusable.
    std::fs::write(dir.join("2026-08-12.json"), r#"{"predictions": []}"#).expect("write");
    assert!(load(&home, "2026-08-12").is_none());
}

// ── scoring ──────────────────────────────────────────────────────────────────

#[test]
fn a_direction_is_right_only_when_the_close_clears_the_band() {
    assert_eq!(score("bullish", Some(1.2)), Outcome::Hit);
    assert_eq!(score("bullish", Some(0.2)), Outcome::Miss);
    assert_eq!(score("bearish", Some(-1.2)), Outcome::Hit);
    assert_eq!(score("bearish", Some(-0.2)), Outcome::Miss);
    assert_eq!(score("neutral", Some(0.2)), Outcome::Hit);
    assert_eq!(score("neutral", Some(1.2)), Outcome::Miss);
}

#[test]
fn the_band_edge_belongs_to_neutral_and_to_nobody_else() {
    // Exactly ±0.5%. One day cannot satisfy two directions, so the boundary
    // has to fall on one side and it falls on neutral's.
    assert_eq!(score("neutral", Some(0.5)), Outcome::Hit);
    assert_eq!(score("neutral", Some(-0.5)), Outcome::Hit);
    assert_eq!(score("bullish", Some(0.5)), Outcome::Miss);
    assert_eq!(score("bearish", Some(-0.5)), Outcome::Miss);
}

#[test]
fn a_prediction_with_no_direction_or_no_price_is_unscored_not_wrong() {
    assert_eq!(score("", Some(3.0)), Outcome::Unscored);
    assert_eq!(score("wobbly", Some(3.0)), Outcome::Unscored);
    assert_eq!(score("bullish", None), Outcome::Unscored);
}

#[test]
fn a_missing_side_or_a_zero_reference_yields_no_percentage() {
    assert_eq!(pct_change(Some(200.0), Some(202.0)), Some(1.0));
    assert_eq!(pct_change(None, Some(202.0)), None);
    assert_eq!(pct_change(Some(200.0), None), None);
    assert_eq!(pct_change(Some(0.0), Some(202.0)), None);
}

#[test]
fn the_tally_counts_hits_against_what_could_be_scored() {
    let rows = review(&sample().predictions, &|t| match t {
        "AAPL" => Some(210.0), // +5% against 200 — bullish, a hit
        _ => Some(1.0),        // TSLA has no reference price, so it cannot score
    });
    assert_eq!(rows[0].outcome, Outcome::Hit);
    assert_eq!(rows[1].outcome, Outcome::Unscored);
    // 1 of 1 scorable, not 1 of 2. An unscorable row must not deflate the score.
    assert_eq!(tally(&rows), (1, 1));
}

// ── the rendered review ──────────────────────────────────────────────────────

fn reviewed_rows() -> Vec<Reviewed> {
    review(&sample().predictions, &|t| match t {
        "AAPL" => Some(210.0),
        _ => None,
    })
}

#[test]
fn the_review_names_the_prediction_the_move_and_the_band() {
    let out = format_review(&reviewed_rows(), "08:30 EDT").join("\n");
    assert!(out.contains("🔁 盤前預測覆盤（08:30 EDT 的判斷）"), "{out}");
    assert!(out.contains("✅ AAPL 盤前看漲 🟢 82% → 實際 +5.00%"), "{out}");
    assert!(out.contains("➖ TSLA"), "{out}");
    assert!(out.contains("命中 1/1"), "{out}");
    assert!(out.contains("±0.5%"), "{out}");
    assert!(out.contains("1 支無法評分"), "{out}");
}

#[test]
fn no_journal_renders_no_review_section_at_all() {
    // Distinct from an empty one: "we kept no record" and "we predicted
    // nothing" are different claims and must not render the same.
    assert!(format_review(&[], "08:30 EDT").is_empty());
}

#[test]
fn the_close_report_puts_the_review_above_the_days_analysis() {
    let rows = vec![Row {
        ticker: "AAPL".into(),
        agreement: Agreement::Consensus,
        sentiment: "bullish".into(),
        confidence: 0.8,
        reason: "strong".into(),
        primary: Opinion {
            sentiment: "bullish".into(),
            confidence: 0.8,
            reason: "strong".into(),
        },
        backup: Opinion {
            sentiment: "bullish".into(),
            confidence: 0.8,
            reason: "strong".into(),
        },
    }];
    let binding = reviewed_rows();
    let out = format_report(
        &rows,
        &ReportContext {
            mode: "eod",
            ticker_count: 1,
            date: "2026-08-12",
            market_time: "16:10 EDT",
            review: &binding,
            review_made_at: "08:30 EDT",
        },
    );
    assert!(out.starts_with("📊 CCT2 收盤報告｜2026-08-12 16:10 EDT"), "{out}");
    let review_at = out.find("🔁 盤前預測覆盤").expect("review present");
    let analysis_at = out.find("🎯 共識訊號").expect("analysis present");
    assert!(review_at < analysis_at, "review must lead:\n{out}");
}

#[test]
fn a_report_with_no_market_time_still_renders_its_date() {
    // The zoneinfo-missing path. Degraded, not broken.
    let out = format_report(
        &[],
        &ReportContext {
            mode: "pre-market",
            ticker_count: 1,
            date: "2026-08-12",
            ..Default::default()
        },
    );
    assert!(out.starts_with("📊 CCT2 盤前報告｜2026-08-12\n"), "{out}");
    assert!(!out.contains("🔁"), "{out}");
}
