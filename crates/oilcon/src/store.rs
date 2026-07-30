//! Backfill policy and Yahoo-only window reads against price-registry.
//!
//! This is the only new logic in the oilcon port: Python's presence check is not
//! safe against a shared `prices` table, so it is replaced rather than translated.

use crate::analysis::Row;
use libsql::Connection;
use price_store::read_window_from_source;

/// Matches `run.py`'s `WINDOW_SIZE` — the one-year trading-day window.
pub const WINDOW_SIZE: i64 = 252;

/// Analytic sufficiency: `classify_oil_trend`'s floor.
pub const MIN_ROWS: usize = 70;

/// Horizon coverage for the one-year extrema `compute_extremes` reports.
pub const MIN_SPAN_DAYS: i64 = 300;

/// Freshness: long weekend plus holidays.
pub const MAX_STALE_DAYS: i64 = 7;

/// Canonical source for the three oil tickers, matching `price-cli`.
pub const YAHOO_SOURCE: &str = "yahoo";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Coverage {
    pub rows: usize,
    pub newest: Option<String>,
    pub span_days: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmptyStoreOnFailedRefresh;

/// Three-condition backfill guard. All three are required:
/// count (analytic floor), freshness (tail gap), span (horizon coverage).
///
/// `today` is passed in — never read inside — so tests pin it and a midnight
/// run classifies staleness on the same day boundary the rendered timestamp uses.
pub fn needs_backfill(cov: &Coverage, today: &str) -> bool {
    if cov.rows < MIN_ROWS {
        return true;
    }
    match &cov.newest {
        None => return true,
        Some(newest) => {
            if is_stale(newest, today) {
                return true;
            }
        }
    }
    if cov.span_days < MIN_SPAN_DAYS {
        return true;
    }
    false
}

/// True when `newest` is older than `today - MAX_STALE_DAYS` (strictly more than
/// seven calendar days behind). Exactly seven days is still fresh.
fn is_stale(newest: &str, today: &str) -> bool {
    match (parse_iso(newest), parse_iso(today)) {
        (Some(n), Some(t)) => civil_days(t) - civil_days(n) > MAX_STALE_DAYS,
        // Unparseable dates are treated as stale so a corrupt row cannot silence backfill.
        _ => true,
    }
}

/// Coverage over a chronologically ascending Yahoo window.
/// `span_days` is the calendar difference between the oldest and newest ISO dates,
/// not `rows - 1` — those coincide only on a gapless daily series.
pub fn coverage(rows: &[Row]) -> Coverage {
    if rows.is_empty() {
        return Coverage {
            rows: 0,
            newest: None,
            span_days: 0,
        };
    }
    let oldest = &rows[0].day;
    let newest = &rows[rows.len() - 1].day;
    let span_days = match (parse_iso(oldest), parse_iso(newest)) {
        (Some(a), Some(b)) => civil_days(b) - civil_days(a),
        _ => 0,
    };
    Coverage {
        rows: rows.len(),
        newest: Some(newest.clone()),
        span_days,
    }
}

/// Newest 252 rows of source `yahoo` only, ascending. Filtered in SQL via
/// `price_store::read_window_from_source` — never a Rust-side filter over
/// `read_window`, which would take "the yahoo subset of the newest 252 of any source".
pub async fn load_window(conn: &Connection, ticker: &str) -> Result<Vec<Row>, String> {
    let stored = read_window_from_source(conn, ticker, YAHOO_SOURCE, WINDOW_SIZE)
        .await
        // turso_util::Error implements neither Display nor std::error::Error, so `{e}`
        // will not compile — but `{e:?}` is not the fallback to reach for. This string
        // ends up in build_snapshot's warning, which is rendered into the delivered
        // message as `[WARN: turso unavailable - …]`, where Python puts `str(exc)`.
        // A Debug dump of the struct would ship `Error { kind: Turso, message: "…" }`
        // to a Telegram reader. `kind_str()` and `message()` are public; use them.
        .map_err(|e| format!("{}: {}", e.kind_str(), e.message()))?;
    Ok(stored
        .into_iter()
        .map(|sp| Row {
            day: sp.date,
            close: sp.close,
        })
        .collect())
}

/// When a history refresh fails: keep stored Yahoo rows if any and let the caller
/// mark the symbol stale. Only an empty store may hard-fail.
pub fn after_failed_refresh(stored: Vec<Row>) -> Result<Vec<Row>, EmptyStoreOnFailedRefresh> {
    if stored.is_empty() {
        Err(EmptyStoreOnFailedRefresh)
    } else {
        Ok(stored)
    }
}

/// Parse `YYYY-MM-DD`. Returns None on any other shape.
fn parse_iso(s: &str) -> Option<(i32, u32, u32)> {
    let mut parts = s.split('-');
    let y: i32 = parts.next()?.parse().ok()?;
    let m: u32 = parts.next()?.parse().ok()?;
    let d: u32 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || m == 0 || m > 12 || d == 0 || d > 31 {
        return None;
    }
    Some((y, m, d))
}

/// Days since civil epoch (proleptic Gregorian), Howard Hinnant algorithm.
fn civil_days(ymd: (i32, u32, u32)) -> i64 {
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

#[cfg(test)]
mod unit {
    use super::*;

    #[test]
    fn civil_days_known_offsets() {
        // 1970-01-01 is day 0 in this epoch convention for unix-ish anchors;
        // check a pair with a known calendar span instead.
        let a = civil_days((2026, 1, 1));
        let b = civil_days((2026, 1, 31));
        assert_eq!(b - a, 30);
        let c = civil_days((2025, 1, 1));
        assert_eq!(a - c, 365); // 2025 is not a leap year
        let d = civil_days((2024, 1, 1));
        assert_eq!(c - d, 366); // 2024 is a leap year
    }
}
