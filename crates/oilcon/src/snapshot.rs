//! Snapshot assembly: translate `build_symbol_snapshot` / `build_snapshot`.
//!
//! Sequence differs from Python because Task 3's coverage guard needs a window
//! before the backfill question can be asked. Fetcher and store are injected.
//!
//! Intentional divergence (populated-store history-failure fallback): a symbol
//! that already has stored Yahoo rows survives a failed history refresh, is
//! marked stale, and continues — where Python aborts the whole snapshot. See
//! `after_failed_refresh`. Task 6's differential will show this if exercised.

use crate::analysis::Row;
use crate::store::{
    after_failed_refresh, coverage, load_window, needs_backfill, EmptyStoreOnFailedRefresh,
    YAHOO_SOURCE,
};
use libsql::Connection;
use market_fetch::yahoo::FetchError;
use price_store::{ensure_schema, upsert, upsert_many, StoredPrice};

/// Matches `run.py`'s `MIN_HISTORY_ROWS`. Below this, WTI aborts; Brent/HO → `rows = None`.
pub const MIN_HISTORY_ROWS: usize = 20;

/// Symbol processing order — insertion order of Python's `SYMBOLS` dict.
pub const SYMBOLS: &[(&str, &str)] = &[("WTI", "CL=F"), ("Brent", "BZ=F"), ("HO", "HO=F")];

/// Per-symbol rows plus the stale flag, matching Python's `SymbolSnapshot`.
#[derive(Debug, Clone, PartialEq)]
pub struct SymbolSnapshot {
    pub rows: Option<Vec<Row>>,
    pub stale: bool,
}

/// Full run snapshot. Field shape differs from Python's `symbols: dict` but
/// carries the same three labels; a hard failure clears all three and sets
/// `warning`, matching `Snapshot(symbols={}, warning=…)`.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub wti: SymbolSnapshot,
    pub brent: SymbolSnapshot,
    pub ho: SymbolSnapshot,
    pub warning: Option<String>,
}

impl Snapshot {
    fn empty_with_warning(warning: String) -> Self {
        Self {
            wti: SymbolSnapshot {
                rows: None,
                stale: false,
            },
            brent: SymbolSnapshot {
                rows: None,
                stale: false,
            },
            ho: SymbolSnapshot {
                rows: None,
                stale: false,
            },
            warning: Some(warning),
        }
    }
}

/// Map market-fetch outcomes the way Python's `parse_chart_response` does:
/// `Upstream` (chart.error) and `NoData` (missing result / falsy closes) both
/// become an empty series. Http and Parse stay errors.
pub fn history_rows_or_empty(result: Result<Vec<Row>, FetchError>) -> Result<Vec<Row>, FetchError> {
    match result {
        Ok(rows) => Ok(rows),
        Err(FetchError::Upstream(_)) | Err(FetchError::NoData) => Ok(Vec::new()),
        Err(e) => Err(e),
    }
}

fn store_err(e: turso_util::Error) -> String {
    format!("{}: {}", e.kind_str(), e.message())
}

async fn write_history(conn: &Connection, ticker: &str, rows: &[Row]) -> Result<(), String> {
    if rows.is_empty() {
        return Ok(());
    }
    let stored: Vec<StoredPrice> = rows
        .iter()
        .map(|r| StoredPrice {
            ticker: ticker.to_string(),
            date: r.day.clone(),
            close: r.close,
            source: YAHOO_SOURCE.to_string(),
        })
        .collect();
    upsert_many(conn, &stored).await.map_err(store_err)
}

/// Build one symbol's snapshot.
///
/// Order (Task 3 guard forces the first read before the question):
/// `load_window` → `coverage` → `needs_backfill` → optional history + `upsert_many`
/// → **re-read the window** → `fetch_latest` → optional `upsert` (+ re-read so the
/// new point is in `rows`, matching Python's `window()` after latest) →
/// `MIN_HISTORY_ROWS` check → `SymbolSnapshot`.
///
/// Reading the window after a successful backfill is deliberate: computing the
/// working set from the fetch payload would skip whatever the upsert committed
/// (window limit, sort order, prior rows).
pub async fn build_symbol_snapshot(
    conn: &Connection,
    label: &str,
    symbol: &str,
    today: &str,
    fetch_history: &dyn Fn(&str) -> Result<Vec<Row>, FetchError>,
    fetch_latest: &dyn Fn(&str) -> Result<Option<Row>, FetchError>,
) -> Result<SymbolSnapshot, String> {
    let mut rows = load_window(conn, symbol).await?;
    let mut history_stale = false;

    if needs_backfill(&coverage(&rows), today) {
        match history_rows_or_empty(fetch_history(symbol)) {
            Ok(history_rows) => {
                write_history(conn, symbol, &history_rows).await?;
                // Re-read — never use `history_rows` as the window.
                rows = load_window(conn, symbol).await?;
            }
            Err(e) => match after_failed_refresh(rows) {
                Ok(kept) => {
                    // Populated store: keep rows, mark stale, continue.
                    // Differs from Python, which aborts unconditionally.
                    rows = kept;
                    history_stale = true;
                }
                Err(EmptyStoreOnFailedRefresh) => {
                    return Err(format!("history fetch failed for {label} - {e}"));
                }
            },
        }
    }

    // `fetch_latest` reaches the stale flag two ways: throw and Ok(None).
    // Only Ok(Some(_)) leaves this path clear and upserts.
    let mut latest_failed = false;
    match fetch_latest(symbol) {
        Ok(Some(r)) => {
            upsert(conn, symbol, &r.day, r.close, YAHOO_SOURCE)
                .await
                .map_err(store_err)?;
            // Python calls `window()` after the latest upsert; without this the
            // new observation is in the store but missing from the snapshot.
            rows = load_window(conn, symbol).await?;
        }
        Ok(None) | Err(_) => {
            latest_failed = true;
        }
    }

    if rows.len() < MIN_HISTORY_ROWS {
        if label == "WTI" {
            return Err(format!(
                "insufficient WTI history ({} rows)",
                rows.len()
            ));
        }
        // Brent/HO short history: rows=None. Python drops stale here
        // (`SymbolSnapshot(rows=None)` defaults stale=False).
        return Ok(SymbolSnapshot {
            rows: None,
            stale: false,
        });
    }

    Ok(SymbolSnapshot {
        rows: Some(rows),
        stale: history_stale || latest_failed,
    })
}

/// Build the full three-symbol snapshot. Hard failures (empty-store history
/// abort, insufficient WTI, store errors) discard any symbols already built
/// and return `warning` only — Python's all-or-nothing model. Writes already
/// committed by earlier symbols stay in the store.
pub async fn build_snapshot(
    conn: &Connection,
    today: &str,
    fetch_history: &dyn Fn(&str) -> Result<Vec<Row>, FetchError>,
    fetch_latest: &dyn Fn(&str) -> Result<Option<Row>, FetchError>,
) -> Snapshot {
    if let Err(e) = ensure_schema(conn).await {
        return Snapshot::empty_with_warning(format!("turso unavailable - {}", store_err(e)));
    }

    let mut wti: Option<SymbolSnapshot> = None;
    let mut brent: Option<SymbolSnapshot> = None;
    let mut ho: Option<SymbolSnapshot> = None;

    for &(label, symbol) in SYMBOLS {
        match build_symbol_snapshot(conn, label, symbol, today, fetch_history, fetch_latest).await
        {
            Ok(snap) => match label {
                "WTI" => wti = Some(snap),
                "Brent" => brent = Some(snap),
                "HO" => ho = Some(snap),
                _ => unreachable!("SYMBOLS only contains WTI/Brent/HO"),
            },
            Err(msg) => return Snapshot::empty_with_warning(msg),
        }
    }

    Snapshot {
        wti: wti.expect("WTI processed"),
        brent: brent.expect("Brent processed"),
        ho: ho.expect("HO processed"),
        warning: None,
    }
}
