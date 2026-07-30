//! Snapshot assembly tests. In-memory libsql only — no network, no live registry.

use market_fetch::yahoo::FetchError;
use oilcon::analysis::{classify_oil_trend, Row};
use oilcon::snapshot::{
    build_snapshot, build_symbol_snapshot, history_rows_or_empty, MIN_HISTORY_ROWS, SYMBOLS,
};
use oilcon::store::{load_window, WINDOW_SIZE};
use price_store::{ensure_schema, upsert};
use std::cell::RefCell;

const TODAY: &str = "2026-07-30";

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

/// `n` ascending daily rows ending on `end` (inclusive).
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
fn spanning_ending(end: &str, n: usize, span: i64) -> Vec<Row> {
    assert!(n >= 2);
    let end_days = civil(parse(end));
    let start_days = end_days - span;
    let mut days: Vec<i64> = (0..n)
        .map(|i| start_days + (i as i64 * span) / (n as i64 - 1))
        .collect();
    days[n - 1] = end_days;
    for i in 1..n {
        if days[i] <= days[i - 1] {
            days[i] = days[i - 1] + 1;
        }
    }
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

async fn seed_yahoo(conn: &libsql::Connection, ticker: &str, rows: &[Row]) {
    for r in rows {
        upsert(conn, ticker, &r.day, r.close, "yahoo")
            .await
            .unwrap();
    }
}

/// Full coverage so needs_backfill is false (70+ rows, fresh, span ≥ 300).
/// Must span ≥ 300 calendar days *inside the 252-row window* load_window returns —
/// 252 dense calendar days only span 251 and still need backfill.
fn adequate_series() -> Vec<Row> {
    spanning_ending(TODAY, 252, 365)
}

fn latest_ok(day: &str, close: f64) -> impl Fn(&str) -> Result<Option<Row>, FetchError> {
    let day = day.to_string();
    move |_| Ok(Some(row(&day, close)))
}

fn latest_none() -> impl Fn(&str) -> Result<Option<Row>, FetchError> {
    |_| Ok(None)
}

fn history_err() -> impl Fn(&str) -> Result<Vec<Row>, FetchError> {
    |_| Err(FetchError::Http("history down".into()))
}

// ── after_failed_refresh halves ────────────────────────────────────────────

#[tokio::test]
async fn history_failure_on_empty_store_aborts_and_discards_earlier_symbols() {
    // WTI succeeds (seeded + no backfill). Brent has empty store, history fails
    // → EmptyStoreOnFailedRefresh → abort. Snapshot is empty + warning; WTI's
    // in-memory SymbolSnapshot is discarded even though CL=F writes remain.
    let c = mem().await;
    seed_yahoo(&c, "CL=F", &adequate_series()).await;

    let hist_calls: RefCell<Vec<String>> = RefCell::new(Vec::new());
    let fetch_history = |sym: &str| -> Result<Vec<Row>, FetchError> {
        hist_calls.borrow_mut().push(sym.to_string());
        // Only Brent/HO reach history (WTI is adequate). Fail them.
        Err(FetchError::Http("history down".into()))
    };
    let fetch_latest = latest_ok(TODAY, 99.0);

    let snap = build_snapshot(&c, TODAY, &fetch_history, &fetch_latest).await;

    assert!(snap.warning.is_some(), "empty-store history failure must abort");
    let w = snap.warning.as_deref().unwrap();
    assert!(
        w.contains("history fetch failed for Brent"),
        "warning must name Brent, got {w}"
    );
    assert!(
        w.contains("history down"),
        "warning must carry the fetch error, got {w}"
    );
    // All-or-nothing: no partial symbol snapshots.
    assert!(snap.wti.rows.is_none());
    assert!(snap.brent.rows.is_none());
    assert!(snap.ho.rows.is_none());
    // WTI's earlier store writes are still committed (order WTI → Brent → HO).
    let wti_stored = load_window(&c, "CL=F").await.unwrap();
    assert!(
        wti_stored.len() >= 252,
        "WTI writes must remain after Brent aborts (window capped at 252)"
    );
}

#[tokio::test]
async fn history_failure_on_populated_store_falls_back_marks_stale_and_continues() {
    // Intentional divergence from Python: 30 stored rows survive a failed
    // refresh, clear MIN_HISTORY_ROWS, and classify as insufficient-history.
    let c = mem().await;
    let thirty = dense_ending("2026-07-20", 30); // stale enough to need backfill
    seed_yahoo(&c, "CL=F", &thirty).await;
    seed_yahoo(&c, "BZ=F", &adequate_series()).await;
    seed_yahoo(&c, "HO=F", &adequate_series()).await;

    let snap = build_snapshot(&c, TODAY, &history_err(), &latest_ok(TODAY, 70.0)).await;

    assert!(
        snap.warning.is_none(),
        "populated-store history failure must NOT abort, got {:?}",
        snap.warning
    );
    let wti_rows = snap.wti.rows.as_ref().expect("WTI must keep stored rows");
    // 30 stored ending 2026-07-20 + latest upsert of TODAY → 31 rows. Both
    // clear MIN_HISTORY_ROWS (20) and sit under classify's 70.
    assert!(
        wti_rows.len() >= 30 && wti_rows.len() <= 31,
        "fallback keeps stored rows (plus at most the latest day), got {}",
        wti_rows.len()
    );
    assert!(
        snap.wti.stale,
        "history-failure fallback marks the symbol stale"
    );
    // end to end: degraded classification, not a lost report
    assert_eq!(classify_oil_trend(wti_rows), "insufficient-history");
    // Other symbols continue.
    assert!(snap.brent.rows.is_some());
    assert!(snap.ho.rows.is_some());
}

// ── fetch_latest stale flag ────────────────────────────────────────────────

#[tokio::test]
async fn throwing_latest_fetch_marks_stale() {
    let c = mem().await;
    seed_yahoo(&c, "CL=F", &adequate_series()).await;
    seed_yahoo(&c, "BZ=F", &adequate_series()).await;
    seed_yahoo(&c, "HO=F", &adequate_series()).await;

    // WTI throws; Brent/HO succeed — mirrors Python's test.
    let calls = RefCell::new(0usize);
    let fetch_latest = |sym: &str| -> Result<Option<Row>, FetchError> {
        let n = {
            let mut c = calls.borrow_mut();
            *c += 1;
            *c
        };
        if n == 1 {
            assert_eq!(sym, "CL=F");
            return Err(FetchError::Http("boom".into()));
        }
        Ok(Some(row(TODAY, 80.0)))
    };
    let fetch_history = history_err(); // must not be called (all adequate)

    let snap = build_snapshot(&c, TODAY, &fetch_history, &fetch_latest).await;
    assert!(snap.warning.is_none());
    assert!(snap.wti.stale, "throwing latest must set stale");
    assert!(!snap.brent.stale, "successful latest must clear stale");
    assert!(!snap.ho.stale);
}

#[tokio::test]
async fn ok_none_latest_fetch_marks_stale() {
    // Python only tests the throwing path; Ok(None) needs its own test.
    let c = mem().await;
    seed_yahoo(&c, "CL=F", &adequate_series()).await;
    seed_yahoo(&c, "BZ=F", &adequate_series()).await;
    seed_yahoo(&c, "HO=F", &adequate_series()).await;

    let snap = build_snapshot(&c, TODAY, &history_err(), &latest_none()).await;
    assert!(snap.warning.is_none());
    assert!(snap.wti.stale, "Ok(None) must set stale just like Err");
    assert!(snap.brent.stale);
    assert!(snap.ho.stale);
    // Successful history is not involved; nothing else sets stale.
}

#[tokio::test]
async fn successful_latest_does_not_set_stale() {
    let c = mem().await;
    seed_yahoo(&c, "CL=F", &adequate_series()).await;
    seed_yahoo(&c, "BZ=F", &adequate_series()).await;
    seed_yahoo(&c, "HO=F", &adequate_series()).await;

    let snap = build_snapshot(&c, TODAY, &history_err(), &latest_ok(TODAY, 70.0)).await;
    assert!(snap.warning.is_none());
    assert!(!snap.wti.stale);
    assert!(!snap.brent.stale);
    assert!(!snap.ho.stale);
}

#[tokio::test]
async fn the_latest_observation_is_present_in_the_snapshot_rows() {
    // The window must be read AFTER the latest upsert, as `run.py` does — its
    // single `window()` call sits below the upsert. Reading before it leaves the
    // new close committed to the store but absent from `rows`, so every rendered
    // number (current_close, today_change_pct, both extremes, pct_below_60d_high,
    // and the classification) would be computed one observation behind.
    //
    // This test exists because that ordering was specified wrongly in the plan and
    // corrected during implementation, and removing the post-latest re-read left
    // all sixteen other tests green.
    let c = mem().await;
    let series = adequate_series();
    // A distinctive close on a date one day past the seeded series.
    let last_seeded = series.last().unwrap().day.clone();
    let next_day = {
        let d = civil(parse(&last_seeded)) + 1;
        let (y, m, dd) = from_civil(d);
        format!("{y:04}-{m:02}-{dd:02}")
    };
    seed_yahoo(&c, "CL=F", &series).await;

    let snap = build_symbol_snapshot(
        &c,
        "WTI",
        "CL=F",
        &next_day,
        &history_err(),
        &latest_ok(&next_day, 123.45),
    )
    .await
    .unwrap();

    let rows = snap.rows.expect("WTI rows");
    let last = rows.last().expect("at least one row");
    assert_eq!(last.day, next_day, "the latest observation must be the newest row");
    assert_eq!(last.close, 123.45, "and carry the close the fetch returned");
    // And it must have come through the store, not been appended in memory.
    let stored = load_window(&c, "CL=F").await.unwrap();
    assert_eq!(stored.last().unwrap().close, 123.45);
}

// ── backfill sequence / re-read ────────────────────────────────────────────

#[tokio::test]
async fn needs_backfill_is_asked_after_window_read_and_window_is_reread_after_backfill() {
    // Empty store → needs_backfill → history is called. Snapshot rows come from
    // the store after upsert (WINDOW_SIZE cap, ascending), not the raw fetch.
    let c = mem().await;

    // 300 history rows — more than WINDOW_SIZE — so a re-read yields 252 and
    // using the fetch payload directly would yield 300.
    let hist = dense_ending(TODAY, 300);
    assert!(hist.len() > WINDOW_SIZE as usize);

    let hist_called = RefCell::new(false);
    let fetch_history = |sym: &str| -> Result<Vec<Row>, FetchError> {
        assert_eq!(sym, "CL=F");
        *hist_called.borrow_mut() = true;
        Ok(hist.clone())
    };
    // latest fails so the post-backfill re-read is the one that supplies rows
    // (no second re-read after a latest write).
    let snap = build_symbol_snapshot(
        &c,
        "WTI",
        "CL=F",
        TODAY,
        &fetch_history,
        &latest_none(),
    )
    .await
    .unwrap();

    assert!(*hist_called.borrow(), "empty store must trigger history fetch");
    let rows = snap.rows.as_ref().unwrap();
    assert_eq!(
        rows.len(),
        WINDOW_SIZE as usize,
        "snapshot must carry the re-read window ({WINDOW_SIZE}), not the raw 300 fetch rows"
    );
    // Ascending: first day is older than last.
    assert!(rows.first().unwrap().day < rows.last().unwrap().day);
    assert!(snap.stale, "latest Ok(None) → stale");
}

#[tokio::test]
async fn adequate_store_skips_history_fetch() {
    let c = mem().await;
    seed_yahoo(&c, "CL=F", &adequate_series()).await;

    let hist_called = RefCell::new(false);
    let fetch_history = |_: &str| -> Result<Vec<Row>, FetchError> {
        *hist_called.borrow_mut() = true;
        Ok(vec![])
    };
    let snap = build_symbol_snapshot(
        &c,
        "WTI",
        "CL=F",
        TODAY,
        &fetch_history,
        &latest_ok(TODAY, 70.0),
    )
    .await
    .unwrap();

    assert!(
        !*hist_called.borrow(),
        "adequate coverage must not call fetch_history"
    );
    assert!(snap.rows.is_some());
    assert!(!snap.stale);
}

// ── MIN_HISTORY_ROWS asymmetry ─────────────────────────────────────────────

#[tokio::test]
async fn nineteen_rows_raises_for_wti_but_none_for_brent() {
    let c = mem().await;
    let short = dense_ending(TODAY, 19);
    assert!(short.len() < MIN_HISTORY_ROWS);

    seed_yahoo(&c, "CL=F", &short).await;
    // Force no backfill by… wait, 19 rows always needs backfill on count.
    // Supply history that itself yields 19 so after upsert we still have 19.
    let fetch_history = |_: &str| -> Result<Vec<Row>, FetchError> { Ok(short.clone()) };

    let wti_err = build_symbol_snapshot(
        &c,
        "WTI",
        "CL=F",
        TODAY,
        &fetch_history,
        &latest_none(),
    )
    .await
    .unwrap_err();
    assert!(
        wti_err.contains("insufficient WTI history (19 rows)"),
        "got {wti_err}"
    );

    let c2 = mem().await;
    seed_yahoo(&c2, "BZ=F", &short).await;
    let brent = build_symbol_snapshot(
        &c2,
        "Brent",
        "BZ=F",
        TODAY,
        &fetch_history,
        &latest_none(),
    )
    .await
    .unwrap();
    assert!(brent.rows.is_none(), "Brent short history → rows=None");
    assert!(!brent.stale, "Python drops stale on the short-history return");
}

#[tokio::test]
async fn twenty_rows_is_enough_for_wti() {
    let c = mem().await;
    let twenty = dense_ending(TODAY, 20);
    seed_yahoo(&c, "CL=F", &twenty).await;
    // 20 < 70 so needs_backfill; return the same 20 from history.
    let fetch_history = |_: &str| -> Result<Vec<Row>, FetchError> { Ok(twenty.clone()) };
    let snap = build_symbol_snapshot(
        &c,
        "WTI",
        "CL=F",
        TODAY,
        &fetch_history,
        &latest_none(),
    )
    .await
    .unwrap();
    assert_eq!(snap.rows.as_ref().unwrap().len(), 20);
}

#[tokio::test]
async fn successful_history_fetch_does_not_set_stale() {
    // Apart from latest Ok(None)/Err and the history-failure fallback, nothing
    // sets stale — a successful history refresh must leave the flag clear when
    // latest also succeeds. Without this, "set stale on successful history" is
    // invisible: every other backfill test pairs history with latest_none.
    let c = mem().await;
    let hist = dense_ending(TODAY, 100);
    let fetch_history = |_: &str| -> Result<Vec<Row>, FetchError> { Ok(hist.clone()) };
    let snap = build_symbol_snapshot(
        &c,
        "WTI",
        "CL=F",
        TODAY,
        &fetch_history,
        &latest_ok(TODAY, 70.0),
    )
    .await
    .unwrap();
    assert!(snap.rows.is_some());
    assert!(
        !snap.stale,
        "successful history + successful latest must not be stale"
    );
}

// ── symbol order ───────────────────────────────────────────────────────────

#[tokio::test]
async fn symbols_are_wti_brent_ho_and_brent_failure_leaves_wti_writes() {
    assert_eq!(
        SYMBOLS,
        &[("WTI", "CL=F"), ("Brent", "BZ=F"), ("HO", "HO=F")]
    );

    let c = mem().await;
    // All empty → each needs backfill. WTI history succeeds; Brent fails.
    let order: RefCell<Vec<String>> = RefCell::new(Vec::new());
    let fetch_history = |sym: &str| -> Result<Vec<Row>, FetchError> {
        order.borrow_mut().push(sym.to_string());
        if sym == "CL=F" {
            Ok(dense_ending(TODAY, 100))
        } else {
            Err(FetchError::Http("brent boom".into()))
        }
    };

    let snap = build_snapshot(&c, TODAY, &fetch_history, &latest_none()).await;
    assert_eq!(
        order.borrow().as_slice(),
        &["CL=F".to_string(), "BZ=F".to_string()],
        "must stop at Brent failure; HO not attempted"
    );
    assert!(snap.warning.as_deref().unwrap().contains("Brent"));
    // WTI's history is committed (100 rows, under WINDOW_SIZE so all kept).
    let stored = load_window(&c, "CL=F").await.unwrap();
    assert_eq!(stored.len(), 100, "WTI history must remain after Brent aborts");
    // Snapshot discarded.
    assert!(snap.wti.rows.is_none());
}

// ── Upstream / NoData → empty series ───────────────────────────────────────

#[test]
fn upstream_and_nodata_both_become_empty_series() {
    assert!(history_rows_or_empty(Err(FetchError::Upstream("Not Found".into())))
        .unwrap()
        .is_empty());
    assert!(history_rows_or_empty(Err(FetchError::NoData))
        .unwrap()
        .is_empty());
}

#[test]
fn http_and_parse_stay_errors() {
    assert!(history_rows_or_empty(Err(FetchError::Http("timeout".into()))).is_err());
    assert!(history_rows_or_empty(Err(FetchError::Parse("bad".into()))).is_err());
}

#[tokio::test]
async fn upstream_history_is_empty_not_abort_for_brent() {
    // Mapping only Upstream (not NoData) would still pass this; the unit test
    // above pins both. This pins that empty history does not abort Brent —
    // it yields rows=None (n/a), not a whole-snapshot failure.
    let c = mem().await;
    let fetch_history = |_: &str| -> Result<Vec<Row>, FetchError> {
        Err(FetchError::Upstream("Not Found".into()))
    };
    // Seed WTI and HO adequate so only Brent is empty + Upstream.
    seed_yahoo(&c, "CL=F", &adequate_series()).await;
    seed_yahoo(&c, "HO=F", &adequate_series()).await;

    let snap = build_snapshot(&c, TODAY, &fetch_history, &latest_ok(TODAY, 70.0)).await;
    assert!(
        snap.warning.is_none(),
        "Upstream→empty must not abort the snapshot, got {:?}",
        snap.warning
    );
    assert!(snap.brent.rows.is_none(), "Brent with empty history → n/a");
    assert!(snap.wti.rows.is_some());
    assert!(snap.ho.rows.is_some());
}

#[tokio::test]
async fn nodata_history_is_empty_not_abort_for_brent() {
    let c = mem().await;
    let fetch_history =
        |_: &str| -> Result<Vec<Row>, FetchError> { Err(FetchError::NoData) };
    seed_yahoo(&c, "CL=F", &adequate_series()).await;
    seed_yahoo(&c, "HO=F", &adequate_series()).await;

    let snap = build_snapshot(&c, TODAY, &fetch_history, &latest_ok(TODAY, 70.0)).await;
    assert!(
        snap.warning.is_none(),
        "NoData→empty must not abort, got {:?}",
        snap.warning
    );
    assert!(snap.brent.rows.is_none());
}

// ── successful path smoke ──────────────────────────────────────────────────

#[tokio::test]
async fn happy_path_three_symbols() {
    let c = mem().await;
    seed_yahoo(&c, "CL=F", &adequate_series()).await;
    seed_yahoo(&c, "BZ=F", &adequate_series()).await;
    seed_yahoo(&c, "HO=F", &adequate_series()).await;

    let snap = build_snapshot(&c, TODAY, &history_err(), &latest_ok(TODAY, 70.0)).await;
    assert!(snap.warning.is_none());
    assert!(snap.wti.rows.as_ref().unwrap().len() >= 252);
    assert!(snap.brent.rows.is_some());
    assert!(snap.ho.rows.is_some());
    assert!(!snap.wti.stale && !snap.brent.stale && !snap.ho.stale);
}


