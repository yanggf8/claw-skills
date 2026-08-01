//! Fetch layer — the only network-facing module.
//! Translates run.py's update_state; injects the fetcher so failure branches
//! are unit-testable without a real network call.

use crate::analysis::Row;
use crate::config::Config;
use market_fetch::yahoo::FetchError;
use std::collections::BTreeMap;

/// Map market-fetch outcomes the way Python's parse_chart_response does:
/// `Upstream` (chart.error) and `NoData` (missing result / falsy closes) both
/// become an empty series, which then produces the "yahoo {SYM}: no rows"
/// warning. Http and Parse stay errors → "yahoo fetch {SYM}: …".
/// Rows per symbol, plus the first fetch error if any symbol failed.
pub type Fetched = (BTreeMap<String, Vec<Row>>, Option<String>);

pub fn yahoo_rows_or_empty(result: Result<Vec<Row>, FetchError>) -> Result<Vec<Row>, FetchError> {
    match result {
        Ok(rows) => Ok(rows),
        Err(FetchError::Upstream(_)) | Err(FetchError::NoData) => Ok(Vec::new()),
        Err(e) => Err(e),
    }
}

/// Per-symbol: fetch (range is the caller's concern), sort ascending by date;
/// on error push `yahoo fetch {SYM}: {err}` and store empty; on empty result
/// push `yahoo {SYM}: no rows`. After the loop, if SMH is missing or empty,
/// return Err with joined warnings (or the primary-symbol fallback message);
/// otherwise return state and joined warnings as Some if any.
pub fn update_state(
    cfg: &Config,
    fetch: &dyn Fn(&str) -> Result<Vec<Row>, FetchError>,
) -> Result<Fetched, String> {
    let mut warnings: Vec<String> = Vec::new();
    let mut state: BTreeMap<String, Vec<Row>> = BTreeMap::new();

    for (key, symbol) in &cfg.symbols {
        let sym = symbol.to_uppercase();
        match yahoo_rows_or_empty(fetch(&sym)) {
            Ok(mut rows) => {
                rows.sort_by(|a, b| a.day.cmp(&b.day));
                if rows.is_empty() {
                    warnings.push(format!("yahoo {sym}: no rows"));
                }
                state.insert(key.clone(), rows);
            }
            Err(exc) => {
                warnings.push(format!("yahoo fetch {sym}: {exc}"));
                state.insert(key.clone(), Vec::new());
            }
        }
    }

    let smh_empty = state.get("SMH").map(|r| r.is_empty()).unwrap_or(true);
    if smh_empty {
        let msg = if warnings.is_empty() {
            "yahoo: no SMH history (primary symbol)".to_string()
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
    Ok((state, warn))
}
