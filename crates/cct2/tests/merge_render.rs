//! Tests for the merge and report layers.
//!
//! The Python is evidence, not specification. The central case here is one it
//! got wrong: a ticker answered by only one model was reported as a consensus.

use cct2::merge::{merge, Agreement};
use cct2::render::{fmt_conf, format_report, ReportContext};

fn tickers(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

fn opinions(json: serde_json::Value) -> serde_json::Value {
    json
}

// ── merge: the three cases must not overlap ──────────────────────────────────

#[test]
fn both_models_agreeing_is_a_consensus_and_averages_their_confidence() {
    let p = opinions(serde_json::json!({"AAPL":{"sentiment":"bullish","confidence":0.9,"reason":"p"}}));
    let b = opinions(serde_json::json!({"AAPL":{"sentiment":"bullish","confidence":0.7,"reason":"b"}}));
    let rows = merge(&tickers(&["AAPL"]), Some(&p), Some(&b));
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].agreement, Agreement::Consensus);
    assert_eq!(rows[0].confidence, 0.8);
}

#[test]
fn both_models_disagreeing_is_a_divergence() {
    let p = opinions(serde_json::json!({"AAPL":{"sentiment":"bullish","confidence":0.9}}));
    let b = opinions(serde_json::json!({"AAPL":{"sentiment":"bearish","confidence":0.8}}));
    let rows = merge(&tickers(&["AAPL"]), Some(&p), Some(&b));
    assert_eq!(rows[0].agreement, Agreement::Diverged);
    // The headline follows the primary; the report shows both sides anyway.
    assert_eq!(rows[0].sentiment, "bullish");
    assert_eq!(rows[0].confidence, 0.9);
}

#[test]
fn one_model_answering_is_solo_and_never_consensus() {
    // The bug this port exists to fix. The Python set `consensus: not
    // both_present`, so this row came out marked as a consensus.
    let p = opinions(serde_json::json!({"AAPL":{"sentiment":"bullish","confidence":0.9}}));
    let rows = merge(&tickers(&["AAPL"]), Some(&p), None);
    assert_eq!(rows[0].agreement, Agreement::Solo);
    assert_ne!(rows[0].agreement, Agreement::Consensus);
}

#[test]
fn a_backup_only_answer_is_also_solo_and_reports_the_backups_view() {
    let b = opinions(serde_json::json!({"AAPL":{"sentiment":"bearish","confidence":0.6,"reason":"b only"}}));
    let rows = merge(&tickers(&["AAPL"]), None, Some(&b));
    assert_eq!(rows[0].agreement, Agreement::Solo);
    assert_eq!(rows[0].sentiment, "bearish");
    assert_eq!(rows[0].confidence, 0.6);
    assert_eq!(rows[0].reason, "b only");
}

#[test]
fn a_ticker_neither_model_mentions_is_dropped() {
    // Not reported as neutral or unknown: a silent model is not a reading.
    let p = opinions(serde_json::json!({"AAPL":{"sentiment":"bullish","confidence":0.9}}));
    let rows = merge(&tickers(&["AAPL", "MSFT"]), Some(&p), None);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].ticker, "AAPL");
}

#[test]
fn a_confidence_without_a_sentiment_is_not_an_answer() {
    let p = opinions(serde_json::json!({"AAPL":{"confidence":0.9}}));
    assert!(merge(&tickers(&["AAPL"]), Some(&p), None).is_empty());
}

#[test]
fn sentiment_case_is_normalised_before_comparing() {
    // A model that answers "Bullish" agrees with one that answers "bullish".
    let p = opinions(serde_json::json!({"AAPL":{"sentiment":"Bullish","confidence":0.8}}));
    let b = opinions(serde_json::json!({"AAPL":{"sentiment":"bullish","confidence":0.8}}));
    assert_eq!(
        merge(&tickers(&["AAPL"]), Some(&p), Some(&b))[0].agreement,
        Agreement::Consensus
    );
}

#[test]
fn divergences_sort_ahead_of_everything_else() {
    let p = opinions(serde_json::json!({
        "AAPL":{"sentiment":"bullish","confidence":0.99},
        "MSFT":{"sentiment":"bullish","confidence":0.10}
    }));
    let b = opinions(serde_json::json!({
        "AAPL":{"sentiment":"bullish","confidence":0.99},
        "MSFT":{"sentiment":"bearish","confidence":0.10}
    }));
    let rows = merge(&tickers(&["AAPL", "MSFT"]), Some(&p), Some(&b));
    // MSFT is the least confident and still comes first, because it is the one
    // asking the reader to decide something.
    assert_eq!(rows[0].ticker, "MSFT");
    assert_eq!(rows[1].ticker, "AAPL");
}

// ── render ───────────────────────────────────────────────────────────────────

#[test]
fn a_solo_row_is_filed_under_its_own_heading_not_under_consensus() {
    let p = opinions(serde_json::json!({"AAPL":{"sentiment":"bullish","confidence":0.9,"reason":"only primary spoke"}}));
    let rows = merge(&tickers(&["AAPL"]), Some(&p), None);
    let out = format_report(&rows, &ReportContext { mode: "eod", ticker_count: 1, date: "2026-07-31", ..Default::default() });
    assert!(out.contains("📊 單一模型"), "got:\n{out}");
    assert!(!out.contains("🎯 共識訊號"), "got:\n{out}");
    // …and says which model it was, so the reader can judge the weight.
    assert!(out.contains("僅主模型"), "got:\n{out}");
}

#[test]
fn the_footer_stops_claiming_a_comparison_that_did_not_happen() {
    // "雙模型對照" printed unconditionally in the Python, including on a run
    // where the backup never answered.
    let p = opinions(serde_json::json!({"AAPL":{"sentiment":"bullish","confidence":0.9}}));
    let rows = merge(&tickers(&["AAPL"]), Some(&p), None);
    let out = format_report(&rows, &ReportContext { mode: "eod", ticker_count: 1, date: "2026-07-31", ..Default::default() });
    assert!(out.contains("單一模型回應"), "got:\n{out}");
    assert!(!out.contains("雙模型對照"), "got:\n{out}");
}

#[test]
fn a_mixed_run_says_how_many_of_each() {
    let p = opinions(serde_json::json!({
        "AAPL":{"sentiment":"bullish","confidence":0.9},
        "MSFT":{"sentiment":"bullish","confidence":0.5}
    }));
    let b = opinions(serde_json::json!({"AAPL":{"sentiment":"bullish","confidence":0.9}}));
    let rows = merge(&tickers(&["AAPL", "MSFT"]), Some(&p), Some(&b));
    let out = format_report(&rows, &ReportContext { mode: "eod", ticker_count: 2, date: "2026-07-31", ..Default::default() });
    assert!(out.contains("1 支雙模型對照，1 支僅單一模型"), "got:\n{out}");
}

#[test]
fn a_full_two_model_run_keeps_the_original_footer() {
    let p = opinions(serde_json::json!({"AAPL":{"sentiment":"bullish","confidence":0.9}}));
    let b = opinions(serde_json::json!({"AAPL":{"sentiment":"bullish","confidence":0.9}}));
    let rows = merge(&tickers(&["AAPL"]), Some(&p), Some(&b));
    assert!(format_report(&rows, &ReportContext { mode: "eod", ticker_count: 1, date: "2026-07-31", ..Default::default() }).contains("雙模型對照"));
}

#[test]
fn no_rows_at_all_renders_the_failure_notice() {
    let out = format_report(&[], &ReportContext { mode: "pre-market", ticker_count: 5, date: "2026-07-31", ..Default::default() });
    assert!(out.contains("⚠️ 無法取得任何分析結果"));
    assert!(out.starts_with("📊 CCT2 盤前報告｜2026-07-31"));
}

#[test]
fn the_mode_selects_the_heading() {
    assert!(format_report(&[], &ReportContext { mode: "pre-market", ticker_count: 1, date: "d", ..Default::default() }).contains("盤前報告"));
    assert!(format_report(&[], &ReportContext { mode: "eod", ticker_count: 1, date: "d", ..Default::default() }).contains("收盤報告"));
}

#[test]
fn a_divergence_shows_both_models_side_by_side() {
    let p = opinions(serde_json::json!({"AAPL":{"sentiment":"bullish","confidence":0.9,"reason":"pr"}}));
    let b = opinions(serde_json::json!({"AAPL":{"sentiment":"bearish","confidence":0.8,"reason":"br"}}));
    let rows = merge(&tickers(&["AAPL"]), Some(&p), Some(&b));
    let out = format_report(&rows, &ReportContext { mode: "eod", ticker_count: 1, date: "2026-07-31", ..Default::default() });
    assert!(out.contains("主模型：看漲 🟢 90% — pr"), "got:\n{out}");
    assert!(out.contains("備用模型：看跌 🔴 80% — br"), "got:\n{out}");
}

// ── fmt_conf ─────────────────────────────────────────────────────────────────

#[test]
fn confidence_truncates_rather_than_rounds() {
    // `int(c * 100)` in the Python. Kept: a model that said 0.789 did not claim
    // 79, and this program should not round its estimate up for it.
    assert_eq!(fmt_conf(0.789), "78%");
    assert_eq!(fmt_conf(0.9), "90%");
    assert_eq!(fmt_conf(1.0), "100%");
    assert_eq!(fmt_conf(0.0), "0%");
}

// ── reason clipping ──────────────────────────────────────────────────────────

#[test]
fn a_long_chinese_reason_clips_by_character_without_panicking() {
    // `reason[:80]` counts characters in Python. Byte-slicing a Rust &str here
    // would panic mid-codepoint, and every one of these reasons is Chinese.
    let long: String = "蘋果財報優於預期，".repeat(20);
    let p = serde_json::json!({"AAPL":{"sentiment":"bullish","confidence":0.9,"reason":long}});
    let rows = merge(&tickers(&["AAPL"]), Some(&p), None);
    let out = format_report(&rows, &ReportContext { mode: "eod", ticker_count: 1, date: "2026-07-31", ..Default::default() });
    let reason_line = out.lines().find(|l| l.contains("AAPL")).unwrap();
    let clipped: String = reason_line.chars().skip_while(|c| *c != '—').skip(2).collect();
    assert_eq!(clipped.chars().count(), 80);
}

// ── json::extract ────────────────────────────────────────────────────────────

use cct2::json::extract;

#[test]
fn a_bare_object_parses() {
    assert_eq!(
        extract(r#"{"AAPL":{"sentiment":"bullish"}}"#).unwrap()["AAPL"]["sentiment"],
        "bullish"
    );
}

#[test]
fn a_json_fence_is_stripped() {
    let reply = "```json\n{\"AAPL\":{\"sentiment\":\"bullish\"}}\n```";
    assert_eq!(extract(reply).unwrap()["AAPL"]["sentiment"], "bullish");
}

#[test]
fn a_bare_fence_is_stripped_too() {
    let reply = "```\n{\"AAPL\":{\"sentiment\":\"bearish\"}}\n```";
    assert_eq!(extract(reply).unwrap()["AAPL"]["sentiment"], "bearish");
}

#[test]
fn prose_around_the_object_is_ignored() {
    let reply = "Here is my analysis:\n{\"AAPL\":{\"sentiment\":\"neutral\"}}\nHope that helps.";
    assert_eq!(extract(reply).unwrap()["AAPL"]["sentiment"], "neutral");
}

#[test]
fn a_trailing_comma_before_a_brace_is_tolerated() {
    // Common enough in model output that rejecting it throws away good replies.
    assert_eq!(
        extract(r#"{"AAPL":{"sentiment":"bullish",},}"#).unwrap()["AAPL"]["sentiment"],
        "bullish"
    );
}

#[test]
fn nested_braces_do_not_end_the_object_early() {
    let v = extract(r#"{"a":{"b":{"c":1}},"d":2}"#).unwrap();
    assert_eq!(v["d"], 2);
}

#[test]
fn text_with_no_object_yields_nothing() {
    assert!(extract("I cannot answer that.").is_none());
}

#[test]
fn an_unbalanced_object_yields_nothing_rather_than_a_guess() {
    assert!(extract(r#"{"AAPL":{"sentiment":"bullish""#).is_none());
}
