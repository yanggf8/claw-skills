//! Backfill policy and Yahoo-only load_window. In-memory libsql only — never the
//! live price-registry. No network.

use oilcon::analysis::Row;
use oilcon::store::{
    after_failed_refresh, coverage, load_window, needs_backfill, Coverage, EmptyStoreOnFailedRefresh,
    MAX_STALE_DAYS, MIN_ROWS, MIN_SPAN_DAYS, WINDOW_SIZE,
};
use price_store::{ensure_schema, upsert};

async fn mem() -> libsql::Connection {
    let db = libsql::Builder::new_local(":memory:").build().await.unwrap();
    let c = db.connect().unwrap();
    ensure_schema(&c).await.unwrap();
    c
}

fn row(day: &str, close: f64) -> Row {
    Row {
        day: day.into(),
        close,
    }
}

/// `n` ascending daily rows ending on `end` (inclusive), consecutive calendar days.
fn dense_ending(end: &str, n: usize) -> Vec<Row> {
    let end_days = civil(parse(end));
    (0..n)
        .map(|i| {
            let days = end_days - (n as i64 - 1 - i as i64);
            let (yy, mm, dd) = from_civil(days);
            row(&format!("{yy:04}-{mm:02}-{dd:02}"), 60.0 + i as f64)
        })
        .collect()
}

/// `n` unique ascending rows ending on `end`, spanning exactly `span` calendar days.
/// Dates are spaced as evenly as possible with collisions advanced forward so the
/// primary key stays unique and the last date is exactly `end`.
fn spanning_ending(end: &str, n: usize, span: i64) -> Vec<Row> {
    assert!(n >= 2);
    let end_days = civil(parse(end));
    let start_days = end_days - span;
    let mut days: Vec<i64> = (0..n)
        .map(|i| start_days + (i as i64 * span) / (n as i64 - 1))
        .collect();
    // Force the last to end_days and de-collide by walking forward.
    days[n - 1] = end_days;
    for i in 1..n {
        if days[i] <= days[i - 1] {
            days[i] = days[i - 1] + 1;
        }
    }
    // If de-collision pushed past end, pack the tail back onto end.
    if days[n - 1] > end_days {
        days[n - 1] = end_days;
        for i in (0..n - 1).rev() {
            if days[i] >= days[i + 1] {
                days[i] = days[i + 1] - 1;
            }
        }
    }
    days.into_iter()
        .enumerate()
        .map(|(i, d)| {
            let (yy, mm, dd) = from_civil(d);
            row(&format!("{yy:04}-{mm:02}-{dd:02}"), 60.0 + i as f64)
        })
        .collect()
}

fn parse(s: &str) -> (i32, u32, u32) {
    let mut p = s.split('-');
    (
        p.next().unwrap().parse().unwrap(),
        p.next().unwrap().parse().unwrap(),
        p.next().unwrap().parse().unwrap(),
    )
}

fn civil(ymd: (i32, u32, u32)) -> i64 {
    let (y, m, d) = ymd;
    let y = y as i64;
    let m = m as i64;
    let d = d as i64;
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn from_civil(z: i64) -> (i32, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let y = y + if m <= 2 { 1 } else { 0 };
    (y as i32, m as u32, d as u32)
}

fn cov(rows: usize, newest: Option<&str>, span_days: i64) -> Coverage {
    Coverage {
        rows,
        newest: newest.map(str::to_string),
        span_days,
    }
}

// ── needs_backfill boundaries ──────────────────────────────────────────────

#[test]
fn empty_coverage_needs_backfill() {
    let today = "2026-07-30";
    assert!(needs_backfill(&cov(0, None, 0), today));
    assert!(needs_backfill(&coverage(&[]), today));
}

#[test]
fn sixty_nine_rows_needs_backfill_seventy_does_not_on_count_alone() {
    // Hold freshness and span at passing values so only the count condition fires.
    let today = "2026-07-30";
    let fresh = "2026-07-30";
    assert!(needs_backfill(&cov(69, Some(fresh), 365), today), "69 < MIN_ROWS");
    assert!(
        !needs_backfill(&cov(70, Some(fresh), 365), today),
        "70 with fresh+span must not backfill"
    );
    assert_eq!(MIN_ROWS, 70);
}

#[test]
fn fresh_but_short_span_needs_backfill() {
    let today = "2026-07-30";
    // 100 rows, newest today, but only 100 calendar days of span.
    assert!(needs_backfill(&cov(100, Some(today), 100), today));
    assert!(
        !needs_backfill(&cov(100, Some(today), 300), today),
        "span of exactly MIN_SPAN_DAYS must pass"
    );
    assert_eq!(MIN_SPAN_DAYS, 300);
}

#[test]
fn long_span_but_stale_needs_backfill() {
    let today = "2026-07-30";
    // 8 days behind: older than today - 7.
    assert!(needs_backfill(&cov(252, Some("2026-07-22"), 365), today));
}

#[test]
fn long_span_but_sparse_two_rows_a_year_apart_needs_backfill() {
    // Span alone cannot reject sparse data — two rows 365 days apart satisfy span
    // while failing every observation threshold. Count catches it.
    let today = "2026-07-30";
    let rows = vec![row("2025-07-30", 60.0), row("2026-07-30", 70.0)];
    let c = coverage(&rows);
    assert_eq!(c.rows, 2);
    assert_eq!(c.span_days, 365);
    assert!(needs_backfill(&c, today), "two rows must fail on count");
}

#[test]
fn exactly_seven_days_old_is_still_fresh_eight_is_stale() {
    // "older than today - 7 days": age 7 is fine, age 8 is not.
    // Mutation: `>` → `>=` on the age comparison turns day-7 red.
    let today = "2026-07-30";
    let day7 = "2026-07-23"; // today - 7
    let day8 = "2026-07-22"; // today - 8
    assert!(
        !needs_backfill(&cov(252, Some(day7), 365), today),
        "exactly 7 days old must still be fresh"
    );
    assert!(
        needs_backfill(&cov(252, Some(day8), 365), today),
        "8 days old must be stale"
    );
    assert_eq!(MAX_STALE_DAYS, 7);
}

#[test]
fn seventy_rows_over_three_hundred_days_does_not_backfill() {
    // Recorded limit, not a bug: 70 rows over a 300-day span averages one
    // observation per ~4 days and still satisfies all three conditions. No fourth
    // density condition is added. Pin the decision so it is not "fixed" later.
    let today = "2026-07-30";
    let newest = "2026-07-30";
    assert!(
        !needs_backfill(&cov(70, Some(newest), 300), today),
        "70 rows / 300-day span / fresh must NOT backfill — recorded limit"
    );
}

#[test]
fn span_days_is_calendar_difference_not_row_count_minus_one() {
    // A series with gaps: 5 rows spanning 40 calendar days. rows-1 = 4, which
    // would make span look tiny; calendar span is 40.
    let rows = vec![
        row("2026-01-01", 60.0),
        row("2026-01-10", 61.0),
        row("2026-01-20", 62.0),
        row("2026-01-30", 63.0),
        row("2026-02-10", 64.0),
    ];
    let c = coverage(&rows);
    assert_eq!(c.rows, 5);
    assert_eq!(c.span_days, 40, "calendar span, not rows-1 (=4)");
    assert_ne!(c.span_days, (c.rows as i64) - 1);
}

// ── load_window provenance filter ──────────────────────────────────────────

#[tokio::test]
async fn interleaved_ticker_returns_only_yahoo_and_a_full_limit_of_them() {
    // Seed 252 yahoo rows plus newer foreign rows so the absolute newest rows
    // are foreign. load_window must still return 252 yahoo rows — the limit
    // applies after the source filter. A Rust-side filter over read_window
    // would return far fewer (only the yahoo subset of the newest 252 overall)
    // and this test would catch it.
    let c = mem().await;
    // 252 yahoo rows spanning a full calendar year (trading-day density),
    // ending 2026-06-30.
    let yahoo = spanning_ending("2026-06-30", 252, 365);
    for r in &yahoo {
        upsert(&c, "CL=F", &r.day, r.close, "yahoo").await.unwrap();
    }
    // 50 newer stooq rows after the yahoo window's end (July + part of August).
    for i in 1..=50 {
        let date = if i <= 31 {
            format!("2026-07-{:02}", i)
        } else {
            format!("2026-08-{:02}", i - 31)
        };
        upsert(&c, "CL=F", &date, 100.0 + i as f64, "stooq")
            .await
            .unwrap();
    }

    let loaded = load_window(&c, "CL=F").await.unwrap();
    assert_eq!(
        loaded.len(),
        252,
        "must return a full 252 yahoo rows even though foreign rows are newer; got {}",
        loaded.len()
    );
    assert_eq!(loaded[0].day, yahoo[0].day);
    assert_eq!(loaded[251].day, yahoo[251].day);
    // Coverage is over the filtered set only — foreign rows do not inflate it.
    let cov = coverage(&loaded);
    assert_eq!(cov.rows, 252);
    assert_eq!(cov.newest.as_deref(), Some("2026-06-30"));
    assert!(cov.span_days >= 300, "year-spanning window, got {}", cov.span_days);
    // 5 days after the newest yahoo row — still fresh under the 7-day budget.
    assert!(!needs_backfill(&cov, "2026-07-05"));
}

#[tokio::test]
async fn load_window_ignores_foreign_source_entirely_when_no_yahoo() {
    let c = mem().await;
    for i in 1..=10 {
        upsert(
            &c,
            "CL=F",
            &format!("2026-07-{:02}", i),
            70.0,
            "stooq",
        )
        .await
        .unwrap();
    }
    let loaded = load_window(&c, "CL=F").await.unwrap();
    assert!(loaded.is_empty(), "foreign-only ticker is invisible, not repaired");
    assert!(needs_backfill(&coverage(&loaded), "2026-07-30"));
}

#[tokio::test]
async fn load_window_empty_store_is_empty() {
    let c = mem().await;
    let loaded = load_window(&c, "CL=F").await.unwrap();
    assert!(loaded.is_empty());
    assert_eq!(WINDOW_SIZE, 252);
}

// ── stale-refresh fallback ─────────────────────────────────────────────────

#[test]
fn failed_refresh_falls_back_to_stored_rows() {
    let stored = dense_ending("2026-07-20", 100);
    let kept = after_failed_refresh(stored.clone()).expect("non-empty store must fall back");
    assert_eq!(kept, stored, "fallback returns the stored Yahoo rows unchanged");
}

#[test]
fn failed_refresh_on_empty_store_hard_fails() {
    let err = after_failed_refresh(vec![]).unwrap_err();
    assert_eq!(err, EmptyStoreOnFailedRefresh);
}

#[tokio::test]
async fn failed_refresh_fallback_is_yahoo_only_by_construction() {
    // Fallback returns what load_window returned, so it is already Yahoo-only.
    let c = mem().await;
    upsert(&c, "CL=F", "2026-01-01", 60.0, "yahoo").await.unwrap();
    upsert(&c, "CL=F", "2026-07-01", 70.0, "stooq").await.unwrap();
    let stored = load_window(&c, "CL=F").await.unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].day, "2026-01-01");
    let kept = after_failed_refresh(stored).unwrap();
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].day, "2026-01-01");
}


