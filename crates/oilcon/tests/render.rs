//! Render-layer tests: format_message, format_record_line, confirmation segments.
//! Strings and behaviours are quoted from oilcon/scripts/run.py.

use oilcon::analysis::Row;
use oilcon::render::{
    format_confirmation_segment, format_message, format_record_line, format_wti_line, fmt_pct,
    fmt_price, Snapshot, SymbolSnapshot,
};

fn series(n: usize, base: f64, step: f64) -> Vec<Row> {
    (0..n)
        .map(|i| Row {
            day: format!("2026-{:02}-{:02}", i / 28 + 1, i % 28 + 1),
            close: base + step * i as f64,
        })
        .collect()
}

fn pair(d0: &str, c0: f64, d1: &str, c1: f64) -> Vec<Row> {
    vec![
        Row {
            day: d0.into(),
            close: c0,
        },
        Row {
            day: d1.into(),
            close: c1,
        },
    ]
}

fn fresh_three(
    wti: Vec<Row>,
    brent: Vec<Row>,
    ho: Vec<Row>,
) -> Snapshot {
    Snapshot {
        wti: SymbolSnapshot {
            rows: Some(wti),
            stale: false,
        },
        brent: SymbolSnapshot {
            rows: Some(brent),
            stale: false,
        },
        ho: SymbolSnapshot {
            rows: Some(ho),
            stale: false,
        },
        warning: None,
    }
}

fn minimal_ok_snapshot() -> Snapshot {
    fresh_three(
        pair("2026-04-14", 77.0, "2026-04-15", 78.2),
        pair("2026-04-14", 80.0, "2026-04-15", 80.64),
        pair("2026-04-14", 2.40, "2026-04-15", 2.45),
    )
}

// ---------------------------------------------------------------------------
// Three record-line refusals — each pinned separately (run.py:221–231)
// ---------------------------------------------------------------------------

#[test]
fn format_record_line_refuses_a_warning() {
    let mut snap = minimal_ok_snapshot();
    snap.warning = Some("latest quote unavailable".into());
    let err = format_record_line(&snap, "2026-04-15 12:00:00 CST").unwrap_err();
    assert_eq!(err.as_str(), "record mode requires fresh data");
}

#[test]
fn format_record_line_refuses_a_stale_symbol() {
    let mut snap = minimal_ok_snapshot();
    snap.ho.stale = true;
    let err = format_record_line(&snap, "2026-04-15 12:00:00 CST").unwrap_err();
    assert_eq!(err.as_str(), "record mode requires non-stale data");
}

#[test]
fn format_record_line_refuses_missing_rows() {
    let mut snap = minimal_ok_snapshot();
    snap.brent.rows = None;
    let err = format_record_line(&snap, "2026-04-15 12:00:00 CST").unwrap_err();
    assert_eq!(err.as_str(), "record mode requires complete confirmation data");
}

// ---------------------------------------------------------------------------
// degraded / ok split — no-uptrend is still ok (run.py:191–194)
// ---------------------------------------------------------------------------

#[test]
fn format_message_status_ok_without_warning() {
    let snap = minimal_ok_snapshot();
    let (_msg, status) = format_message(&snap, "2026-04-15 12:00");
    assert_eq!(status, "ok");
}

#[test]
fn format_message_status_degraded_only_when_warning_set() {
    let mut snap = minimal_ok_snapshot();
    snap.warning = Some("latest quote unavailable".into());
    let (msg, status) = format_message(&snap, "2026-04-15 12:00");
    assert_eq!(status, "degraded");
    assert!(msg.contains("[WARN: latest quote unavailable]"));
}

#[test]
fn no_uptrend_classification_with_no_warning_is_still_ok() {
    // Falling series → no-uptrend; status stays ok because status only tracks warning.
    let falling: Vec<Row> = (0..90)
        .map(|i| Row {
            day: format!("d{i:03}"),
            close: 100.0 - 0.4 * i as f64,
        })
        .collect();
    let snap = fresh_three(
        falling,
        pair("2026-04-14", 80.0, "2026-04-15", 80.64),
        pair("2026-04-14", 2.40, "2026-04-15", 2.45),
    );
    let (msg, status) = format_message(&snap, "2026-04-15 12:00");
    assert_eq!(status, "ok");
    assert!(msg.contains("OIL-TREND: no-uptrend"));
}

// ---------------------------------------------------------------------------
// OIL-TREND threshold is 50, not 70; 50–69 renders insufficient-history
// ---------------------------------------------------------------------------

#[test]
fn oil_trend_line_absent_below_fifty_rows() {
    let snap = fresh_three(
        series(49, 60.0, 0.2),
        pair("2026-04-14", 80.0, "2026-04-15", 80.64),
        pair("2026-04-14", 2.40, "2026-04-15", 2.45),
    );
    let (msg, _) = format_message(&snap, "2026-04-15 12:00");
    assert!(
        !msg.contains("OIL-TREND:"),
        "49 rows must not render the OIL-TREND line, got:\n{msg}"
    );
}

#[test]
fn oil_trend_line_at_fifty_to_sixty_nine_renders_insufficient_history() {
    // Threshold for the line is 50; classify_oil_trend still needs 70, so the
    // state between 50 and 69 is insufficient-history.
    let snap = fresh_three(
        series(60, 60.0, 0.2),
        pair("2026-04-14", 80.0, "2026-04-15", 80.64),
        pair("2026-04-14", 2.40, "2026-04-15", 2.45),
    );
    let (msg, _) = format_message(&snap, "2026-04-15 12:00");
    assert!(
        msg.contains("OIL-TREND: insufficient-history"),
        "60 rows must render OIL-TREND with insufficient-history, got:\n{msg}"
    );
}

// ---------------------------------------------------------------------------
// Display comparator (>=) disagrees with classifier (>) at equality
// ---------------------------------------------------------------------------

#[test]
fn oil_trend_at_equal_price_reads_rollover_above() {
    // Task 1 fixed-point fixture: 80 rows of 60.0+0.2i, last close 70.8.
    // classify → rollover (strict > fails); display → "above" (>= holds).
    let mut rows = series(80, 60.0, 0.2);
    rows.last_mut().unwrap().close = 70.8;
    let snap = fresh_three(
        rows,
        pair("2026-04-14", 80.0, "2026-04-15", 80.64),
        pair("2026-04-14", 2.40, "2026-04-15", 2.45),
    );
    let (msg, _) = format_message(&snap, "2026-04-15 12:00");
    // Exact line verified against the Python on this fixture.
    assert!(
        msg.contains(
            "OIL-TREND: rollover (WTI 70.80, above 50MA 70.80, 50MA rising, 6.3% vs 60d-high)"
        ),
        "equal-price line must read 'rollover … above', got:\n{msg}"
    );
}

// ---------------------------------------------------------------------------
// Confirmation: en dash for flat, n/a for short history
// ---------------------------------------------------------------------------

#[test]
fn flat_confirmation_renders_en_dash() {
    // WTI flat (change 0) and HO flat → en dash (U+2013) for both.
    // Brent rises → also en dash because wti_change == 0.
    let snap = fresh_three(
        pair("2026-04-14", 77.0, "2026-04-15", 77.0),
        pair("2026-04-14", 80.0, "2026-04-15", 80.5),
        pair("2026-04-14", 2.40, "2026-04-15", 2.40),
    );
    let (msg, status) = format_message(&snap, "2026-04-15 12:00");
    assert_eq!(status, "ok");
    // en dash is U+2013, not ASCII hyphen-minus
    assert!(
        msg.contains("確認：Brent – (+0.6%)   HO – (+0.0%)"),
        "expected en-dash confirmation, got:\n{msg}"
    );
}

#[test]
fn confirmation_symbol_with_short_history_renders_na() {
    let snap = Snapshot {
        wti: SymbolSnapshot {
            rows: Some(pair("2026-04-14", 77.0, "2026-04-15", 78.2)),
            stale: false,
        },
        brent: SymbolSnapshot {
            rows: None,
            stale: false,
        },
        ho: SymbolSnapshot {
            rows: Some(pair("2026-04-14", 2.40, "2026-04-15", 2.45)),
            stale: true,
        },
        warning: None,
    };
    let (msg, status) = format_message(&snap, "2026-04-15 12:00");
    assert_eq!(status, "ok");
    assert!(
        msg.contains("確認：Brent n/a   HO ✓ (+2.1%) (stale)"),
        "expected n/a + stale HO, got:\n{msg}"
    );
}

// ---------------------------------------------------------------------------
// Exact message and record-line shapes
// ---------------------------------------------------------------------------

#[test]
fn format_message_exact_shape() {
    let snap = minimal_ok_snapshot();
    let (msg, status) = format_message(&snap, "2026-04-15 12:00");
    assert_eq!(status, "ok");
    assert!(msg.starts_with("🛢️ OILCON 情報\n"));
    assert!(msg.contains("WTI: $78.20 (+1.6%)"));
    assert!(msg.contains("  高 $78.20 (2026-04-15, 0日前, +0.0%)"));
    assert!(msg.contains("  低 $77.00 (2026-04-14, 1日前, +1.6% 離低點)"));
    assert!(msg.contains("確認：Brent ✓ (+0.8%)   HO ✓ (+2.1%)"));
    assert!(msg.contains("更新：2026-04-15 12:00"));
    assert!(!msg.contains("OIL-TREND:"), "2 rows must not emit OIL-TREND");
}

#[test]
fn format_record_line_exact_shape() {
    let snap = minimal_ok_snapshot();
    let line = format_record_line(&snap, "2026-04-15 12:00:00 CST").unwrap();
    assert_eq!(
        line,
        "2026-04-15 12:00:00 CST  WTI 78.20  high 78.20@2026-04-15 (+0.0%)  low 77.00@2026-04-14 (+1.6%)  BZ +0.8% HO +2.1%"
    );
}

#[test]
fn format_wti_line_appends_stale_suffix() {
    let snap = SymbolSnapshot {
        rows: Some(pair("2026-04-14", 77.0, "2026-04-15", 78.2)),
        stale: true,
    };
    let (line, _) = format_wti_line(&snap).unwrap();
    assert_eq!(line, "WTI: $78.20 (+1.6%) (stale)");
}

#[test]
fn format_wti_line_refuses_none_rows() {
    let snap = SymbolSnapshot {
        rows: None,
        stale: false,
    };
    let err = format_wti_line(&snap).unwrap_err();
    assert_eq!(err.as_str(), "WTI rows are required");
}

#[test]
fn fmt_price_and_fmt_pct_match_python() {
    assert_eq!(fmt_price(78.2), "$78.20");
    assert_eq!(fmt_pct(1.558441558441562), "+1.6%");
    assert_eq!(fmt_pct(-0.5), "-0.5%");
    assert_eq!(fmt_pct(0.0), "+0.0%");
}

#[test]
fn format_confirmation_segment_direct() {
    let brent = SymbolSnapshot {
        rows: Some(pair("2026-04-14", 80.0, "2026-04-15", 80.64)),
        stale: false,
    };
    // WTI change +1.558… same sign as Brent +0.8 → ✓
    assert_eq!(
        format_confirmation_segment("Brent", &brent, 1.558441558441562),
        "Brent ✓ (+0.8%)"
    );
    let none = SymbolSnapshot {
        rows: None,
        stale: false,
    };
    assert_eq!(
        format_confirmation_segment("Brent", &none, 1.0),
        "Brent n/a"
    );
}
