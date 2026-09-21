//! Fetch layer — the only network-facing module in this port.
//! Translates run.py's `fetch_all` (lines 224–237). The fetcher is injected so
//! failure branches are unit-testable without a real network call.
//!
//! Map market-fetch outcomes the way Python's `fred_fetch.parse_csv` does:
//! `CreditError::NoData` (header-only / all-`.` / empty body) becomes an empty
//! series, which then produces the "{SERIES_ID}: no rows" warning. Http and
//! Parse stay errors → "fetch {SERIES_ID}: …". Without this mapping the same
//! upstream response yields "fetch {id}: no usable observations" instead of
//! "{id}: no rows", and Task 5's character-level differential fails.

use crate::analysis::Obs;
use market_fetch::fred::{build_url, parse_fred_csv, CreditError};
use std::collections::BTreeMap;
use std::time::Duration;

/// FRED public CSV endpoint. Same host path as fred_fetch.py:37.
pub const FRED_CSV_BASE: &str = "https://fred.stlouisfed.org/graph/fredgraph.csv";

/// Start date always sent via market-fetch's `build_url` (`cosd=`). Python omits
/// it; measured 2026-07-29 row counts are identical with/without for all seven
/// series. Kept for full-history safety if FRED's default window ever changes.
pub const FRED_START_DATE: &str = "1900-01-01";

/// fred_fetch.py:32 — FIXED composite. FRED refuses a bare `nullclaw/1.0` by
/// hanging the connection (symptom: timeout, not 4xx). Matching is on the
/// leading token; `curl/8.5.0` is on the allowlist.
pub const USER_AGENT: &str = "curl/8.5.0 nullclaw/1.0";

/// fred_fetch.py:33  DEFAULT_TIMEOUT = 20
pub const FETCH_TIMEOUT_SECS: u64 = 20;

/// Headers the live FRED transport sets. Pure so tests can assert the User-Agent
/// without a network call.
/// Observations per series, plus the first fetch error if any series failed.
pub type Fetched = (BTreeMap<String, Vec<Obs>>, Option<String>);

pub fn fred_request_headers() -> &'static [(&'static str, &'static str)] {
    &[("User-Agent", USER_AGENT)]
}

/// Live FRED series fetch. Uses market-fetch for URL + CSV parse; sets the fixed
/// User-Agent. Call only from the binary / integration paths — unit tests inject
/// a stub into `fetch_all` instead.
pub fn live_fetch(series_id: &str) -> Result<Vec<Obs>, CreditError> {
    let url = build_url(FRED_CSV_BASE, series_id, FRED_START_DATE);
    let mut req = claw_core::http::agent(Duration::from_secs(FETCH_TIMEOUT_SECS)).get(&url);
    for (k, v) in fred_request_headers() {
        req = req.set(k, v);
    }
    let body = req
        .call()
        .map_err(|e| CreditError::Http(e.to_string()))?
        .into_string()
        .map_err(|e| CreditError::Http(e.to_string()))?;
    let rows = parse_fred_csv(series_id, &body)?;
    Ok(rows
        .into_iter()
        .map(|o| Obs {
            day: o.date,
            value: o.value,
        })
        .collect())
}

/// Map market-fetch outcomes the way Python's parse_csv does: `NoData` becomes
/// an empty series (→ "{id}: no rows"); `Http` and `Parse` stay errors
/// (→ "fetch {id}: …").
pub fn fred_rows_or_empty(
    result: Result<Vec<Obs>, CreditError>,
) -> Result<Vec<Obs>, CreditError> {
    match result {
        Ok(rows) => Ok(rows),
        Err(CreditError::NoData) => Ok(Vec::new()),
        Err(e) => Err(e),
    }
}

/// Per series (run.py:227-234): fetch; on error push `fetch {SERIES_ID}: {err}`
/// and store empty; on empty result push `{SERIES_ID}: no rows`.
/// After the loop (run.py:235-237): hard-fail only if `core_pce` is empty, with
/// the joined warnings or the primary-series fallback message.
///
/// A failure in an unused context series still degrades (warning is Some) even
/// though classify never reads that series.
pub fn fetch_all(
    series: &[(String, String)],
    fetch: &dyn Fn(&str) -> Result<Vec<Obs>, CreditError>,
) -> Result<Fetched, String> {
    let mut warnings: Vec<String> = Vec::new();
    let mut out: BTreeMap<String, Vec<Obs>> = BTreeMap::new();

    for (key, series_id) in series {
        match fred_rows_or_empty(fetch(series_id)) {
            Ok(rows) => {
                if rows.is_empty() {
                    // run.py:231  f"{series_id}: no rows"
                    warnings.push(format!("{series_id}: no rows"));
                }
                out.insert(key.clone(), rows);
            }
            Err(exc) => {
                // run.py:233  f"fetch {series_id}: {exc}"
                warnings.push(format!("fetch {series_id}: {exc}"));
                out.insert(key.clone(), Vec::new());
            }
        }
    }

    // run.py:235  if not out.get("core_pce"):
    let core_pce_empty = out.get("core_pce").map(|r| r.is_empty()).unwrap_or(true);
    if core_pce_empty {
        // run.py:236  raise RuntimeError("; ".join(warnings) or "FRED: no core PCE …")
        let msg = if warnings.is_empty() {
            "FRED: no core PCE (PCEPILFE) — primary series".to_string()
        } else {
            warnings.join("; ")
        };
        return Err(msg);
    }

    let warn = if warnings.is_empty() {
        None
    } else {
        Some(warnings.join("; "))
    };
    Ok((out, warn))
}
