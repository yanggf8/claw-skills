use chipcon::analysis::{Details, Status};
use chipcon::config::Config;
use chipcon::render::{format_message, record_line};

/// Built from the real contents of chipcon/config.json (three symbols, five
/// manual_events, position_label as in that file).
fn cfg() -> Config {
    Config {
        symbols: vec![
            ("SMH".into(), "SMH".into()),
            ("QQQ".into(), "QQQ".into()),
            ("SOXX".into(), "SOXX".into()),
        ],
        position_label: "SMH semiconductor momentum observation".into(),
        manual_events: vec![
            "NVDA / AVGO / AMD / MU guidance".into(),
            "TSMC monthly revenue".into(),
            "Microsoft / Amazon / Google / Meta capex guidance".into(),
            "Export-control escalation".into(),
            "SpaceX IPO / index-flow liquidity drain".into(),
        ],
    }
}

fn details() -> Details {
    Details {
        day: "2026-07-29".into(), current: 250.0,
        ma20: Some(240.0), ma50: Some(230.0), rising20: Some(true),
        smh5: Some(1.5), qqq5: Some(2.0), soxx5: Some(1.0),
        rel_qqq5: Some(-0.5), rel_soxx5: Some(0.5),
        down_days: 1, distance20: Some(4.17), distance50: Some(8.70),
        rows: 60, reasons: vec![],
    }
}

#[test]
fn a_warning_makes_the_skill_status_degraded_but_not_the_classification() {
    let (_, s) = format_message(Status::Ok, &details(), &cfg(), Some("yahoo SOXX: no rows"));
    assert_eq!(s, "degraded");
    let (_, s) = format_message(Status::Ok, &details(), &cfg(), None);
    assert_eq!(s, "ok");
}

#[test]
fn a_red_classification_without_a_warning_is_still_skill_status_ok() {
    // RED is a market signal, not a skill failure. Reporting it as degraded
    // would make the scheduler retry a perfectly successful run.
    let (_, s) = format_message(Status::Red, &details(), &cfg(), None);
    assert_eq!(s, "ok");
}

#[test]
fn the_message_is_an_observation_not_an_exit_instruction() {
    let (m, _) = format_message(Status::Red, &details(), &cfg(), None);
    assert!(m.contains("SIGNAL-ONLY"), "{m}");
    assert!(!m.contains("賣出") && !m.contains("SELL"), "must not instruct: {m}");
}

#[test]
fn insufficient_history_renders_the_row_count_not_the_indicators() {
    let mut d = details();
    d.rows = 12;
    let (m, _) = format_message(Status::InsufficientHistory, &d, &cfg(), None);
    assert!(m.contains("12 / 20 needed"), "{m}");
    assert!(!m.contains("50DMA"), "indicator block must be skipped: {m}");
}

#[test]
fn reasons_are_listed_and_their_absence_says_trend_intact() {
    let mut d = details();
    d.reasons = vec!["SMH below 50DMA".into(), "20DMA below 50DMA".into()];
    let (m, _) = format_message(Status::Red, &d, &cfg(), None);
    assert!(m.contains("- SMH below 50DMA") && m.contains("- 20DMA below 50DMA"), "{m}");
    let (m, _) = format_message(Status::Ok, &details(), &cfg(), None);
    assert!(m.contains("trend intact"), "{m}");
}

#[test]
fn record_line_takes_the_clock_as_a_parameter() {
    let l = record_line(Status::Red, &details(), None, "2026-07-29 05:30:00 CST");
    assert!(l.starts_with("2026-07-29 05:30:00 CST CHIPCON RED "), "{l}");
    assert!(l.contains("SMH=250.00"), "{l}");
    assert!(l.ends_with("warning=-"), "no warning renders as a dash: {l}");
}

#[test]
fn record_line_for_insufficient_history_reports_rows_only() {
    let mut d = details();
    d.rows = 3;
    let l = record_line(Status::InsufficientHistory, &d, Some("boom"), "2026-07-29 05:30:00 CST");
    assert!(l.contains("CHIPCON INSUFFICIENT_HISTORY rows=3"), "{l}");
    assert!(l.ends_with("warning=boom"), "{l}");
    assert!(!l.contains("ma20="), "indicator fields must be omitted: {l}");
}
