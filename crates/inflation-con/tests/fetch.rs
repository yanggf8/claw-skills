//! Fetch-layer tests. Expectations are quoted from
//! inflation-con/scripts/run.py `fetch_all` (lines 224–237) and
//! inflation-con/scripts/fred_fetch.py `USER_AGENT` (line 32).
//! No real network call — the inject seam is the only transport.

use inflation_con::analysis::Obs;
use inflation_con::fetch::{fetch_all, fred_request_headers, USER_AGENT};
use market_fetch::fred::CreditError;

/// Default series map order from run.py:55-63 / config::DEFAULT_SERIES.
fn default_series() -> Vec<(String, String)> {
    vec![
        ("core_pce".into(), "PCEPILFE".into()),
        ("core_cpi".into(), "CPILFESL".into()),
        ("headline_pce".into(), "PCEPI".into()),
        ("headline_cpi".into(), "CPIAUCSL".into()),
        ("breakeven_10y".into(), "T10YIE".into()),
        ("real_yield_10y".into(), "DFII10".into()),
        ("nominal_10y".into(), "DGS10".into()),
    ]
}

fn rows(n: usize) -> Vec<Obs> {
    (0..n)
        .map(|i| Obs {
            day: format!("2026-{:02}-01", i % 12 + 1),
            value: 100.0 + i as f64,
        })
        .collect()
}

/// run.py:224-237 — happy path returns data and no warning.
#[test]
fn successful_fetch_returns_all_series_without_warning() {
    let (state, warn) = fetch_all(&default_series(), &|_| Ok(rows(12))).unwrap();
    assert!(warn.is_none());
    assert_eq!(state["core_pce"].len(), 12);
    assert_eq!(state["core_cpi"].len(), 12);
    assert_eq!(state["nominal_10y"].len(), 12);
}

/// run.py:230-231 + 235-236 — empty core_pce is a hard error.
/// wording: "{series_id}: no rows"
#[test]
fn empty_core_pce_is_a_hard_error() {
    let e = fetch_all(&default_series(), &|sid| {
        if sid == "PCEPILFE" {
            Ok(vec![])
        } else {
            Ok(rows(12))
        }
    })
    .unwrap_err();
    // run.py:231  warnings.append(f"{series_id}: no rows")
    // Needle: contains("PCEPILFE: no rows") also matches "fetch PCEPILFE: no rows".
    assert!(
        e.contains("PCEPILFE: no rows"),
        "empty primary must use the no-rows wording: {e}"
    );
    assert!(
        !e.contains("fetch PCEPILFE:"),
        "empty must not look like a transport error: {e}"
    );
}

/// run.py:232-234 + 235-236 — erroring core_pce is a hard error.
/// wording: "fetch {series_id}: {exc}"
#[test]
fn erroring_core_pce_is_a_hard_error() {
    let e = fetch_all(&default_series(), &|sid| {
        if sid == "PCEPILFE" {
            Err(CreditError::Http("boom".into()))
        } else {
            Ok(rows(12))
        }
    })
    .unwrap_err();
    // run.py:233  warnings.append(f"fetch {series_id}: {exc}")
    assert!(
        e.contains("fetch PCEPILFE:"),
        "erroring primary must use the fetch-error wording: {e}"
    );
}

/// run.py:236 fallback when core_pce is empty/missing and there are no warnings
/// (series map omits the primary key entirely).
#[test]
fn missing_core_pce_key_uses_the_primary_series_fallback_message() {
    // No core_pce entry → out.get("core_pce") is falsy, warnings stays empty.
    let series = vec![("core_cpi".into(), "CPILFESL".into())];
    let e = fetch_all(&series, &|_| Ok(rows(12))).unwrap_err();
    // run.py:236  "FRED: no core PCE (PCEPILFE) — primary series"
    assert_eq!(e, "FRED: no core PCE (PCEPILFE) — primary series");
}

/// run.py:232-237 — a secondary (confirmation) failure warns and continues.
/// test_run.py:216-226 pins DGS10 but any non-primary works; use core_cpi here.
#[test]
fn a_failing_secondary_series_only_warns() {
    let (state, warn) = fetch_all(&default_series(), &|sid| {
        if sid == "CPILFESL" {
            Err(CreditError::Http("fred down".into()))
        } else {
            Ok(rows(12))
        }
    })
    .unwrap();
    assert_eq!(state["core_pce"].len(), 12);
    assert!(state["core_cpi"].is_empty());
    let w = warn.expect("a secondary failure must warn");
    // run.py:233  f"fetch {series_id}: {exc}"
    assert!(
        w.contains("fetch CPILFESL:"),
        "secondary error wording must match Python: {w}"
    );
}

/// run.py:230-231 — empty secondary uses the "no rows" wording, not "fetch …".
#[test]
fn an_empty_secondary_warns_with_the_no_rows_wording() {
    let (_, warn) = fetch_all(&default_series(), &|sid| {
        if sid == "CPILFESL" {
            Ok(vec![])
        } else {
            Ok(rows(12))
        }
    })
    .unwrap();
    let w = warn.expect("an empty secondary must warn");
    // run.py:231  f"{series_id}: no rows"
    assert!(
        w.contains("CPILFESL: no rows"),
        "wording must match the Python: {w}"
    );
    assert!(
        !w.contains("fetch CPILFESL:"),
        "empty must not look like a transport error: {w}"
    );
}

/// run.py:227-234 — unused context series (headline_pce / real_yield_10y /
/// nominal_10y) still degrade the run even though classify never reads them.
/// Mirrors test_run.py:216-226 (DGS10 / nominal_10y).
#[test]
fn a_context_series_failure_still_degrades() {
    let (state, warn) = fetch_all(&default_series(), &|sid| {
        if sid == "DGS10" {
            Err(CreditError::Http("fred down".into()))
        } else {
            Ok(rows(12))
        }
    })
    .unwrap();
    assert_eq!(state["core_pce"].len(), 12, "primary succeeded");
    assert!(
        state["nominal_10y"].is_empty(),
        "failed context series stores empty"
    );
    let w = warn.expect("context failure must still degrade");
    assert!(w.contains("DGS10"), "warning names the series id: {w}");
    assert!(
        w.contains("fetch DGS10:"),
        "uses the fetch-error wording: {w}"
    );
}

/// run.py:227 iterates series_map in insertion/config order; warnings are
/// appended then joined with "; ". A BTreeMap of keys would reorder alphabetically
/// (CPIAUCSL before PCEPILFE). Pin config order on the fatal path where the
/// joined text becomes the RuntimeError / failed message.
#[test]
fn warnings_follow_config_order_not_alphabetical_order() {
    let e = fetch_all(&default_series(), &|_| Ok(vec![])).unwrap_err();
    // All seven empty → seven "SERIES_ID: no rows" joined in DEFAULT_SERIES order.
    // Search full "ID: no rows" tokens — bare "PCEPI" is a prefix of "PCEPILFE".
    let pce = e.find("PCEPILFE: no rows").expect("PCEPILFE missing");
    let cpi = e.find("CPILFESL: no rows").expect("CPILFESL missing");
    let headline_pce = e.find("PCEPI: no rows").expect("PCEPI missing");
    let headline_cpi = e.find("CPIAUCSL: no rows").expect("CPIAUCSL missing");
    let be = e.find("T10YIE: no rows").expect("T10YIE missing");
    let real = e.find("DFII10: no rows").expect("DFII10 missing");
    let nom = e.find("DGS10: no rows").expect("DGS10 missing");
    assert!(
        pce < cpi
            && cpi < headline_pce
            && headline_pce < headline_cpi
            && headline_cpi < be
            && be < real
            && real < nom,
        "config order, not alphabetical: {e}"
    );
    // Alphabetical would put CPIAUCSL before PCEPILFE — prove that does not happen.
    assert!(
        pce < headline_cpi,
        "PCEPILFE must precede CPIAUCSL (config order): {e}"
    );
}

/// market-fetch returns CreditError::NoData for header-only / all-`.` / empty
/// body CSV; Python's parse_csv returns []. Map NoData → empty so the warning
/// is "{id}: no rows", not "fetch {id}: no usable observations".
/// Http and Parse must stay on the fetch-error path.
///
/// Needle width: contains("PCEPILFE: no rows") also matches
/// "fetch PCEPILFE: no rows", so every wording assert has a negative clause.
#[test]
fn nodata_maps_to_empty_while_http_and_parse_stay_errors() {
    // --- NoData → empty-result / no-rows wording ---
    let e = fetch_all(&default_series(), &|sid| {
        if sid == "PCEPILFE" {
            Err(CreditError::NoData)
        } else {
            Ok(rows(12))
        }
    })
    .unwrap_err();
    assert!(
        e.contains("PCEPILFE: no rows"),
        "NoData must use the no-rows wording: {e}"
    );
    assert!(
        !e.contains("fetch PCEPILFE:"),
        "NoData must not use the fetch-error wording: {e}"
    );
    assert!(
        !e.contains("no usable observations"),
        "CreditError::NoData Display must not leak: {e}"
    );

    // Secondary NoData also degrades with no-rows, not fetch-error.
    let (_, warn) = fetch_all(&default_series(), &|sid| {
        if sid == "CPILFESL" {
            Err(CreditError::NoData)
        } else {
            Ok(rows(12))
        }
    })
    .unwrap();
    let w = warn.expect("secondary NoData must warn");
    assert!(
        w.contains("CPILFESL: no rows"),
        "secondary NoData must use no-rows: {w}"
    );
    assert!(
        !w.contains("fetch CPILFESL:"),
        "secondary NoData must not look like a transport error: {w}"
    );

    // --- Http stays fetch-error wording ---
    let e = fetch_all(&default_series(), &|sid| {
        if sid == "PCEPILFE" {
            Err(CreditError::Http("boom".into()))
        } else {
            Ok(rows(12))
        }
    })
    .unwrap_err();
    assert!(
        e.contains("fetch PCEPILFE:"),
        "Http must use the fetch-error wording: {e}"
    );
    // Positive-only "PCEPILFE: no rows" would also match "fetch PCEPILFE: no rows";
    // require the fetch prefix and reject a bare no-rows message.
    assert!(
        !e.starts_with("PCEPILFE: no rows") && e.contains("fetch PCEPILFE:"),
        "Http must not collapse to the no-rows wording: {e}"
    );

    // --- Parse stays fetch-error wording ---
    let e = fetch_all(&default_series(), &|sid| {
        if sid == "PCEPILFE" {
            Err(CreditError::Parse("bad csv".into()))
        } else {
            Ok(rows(12))
        }
    })
    .unwrap_err();
    assert!(
        e.contains("fetch PCEPILFE:"),
        "Parse must use the fetch-error wording: {e}"
    );
    assert!(
        !e.starts_with("PCEPILFE: no rows") && e.contains("fetch PCEPILFE:"),
        "Parse must not collapse to the no-rows wording: {e}"
    );
}

/// fred_fetch.py:32  USER_AGENT = "curl/8.5.0 nullclaw/1.0"
/// FRED matches on the leading token; bare "nullclaw/1.0" hangs the connection.
/// Assert the exact header the live fetcher sends — do not trust market-fetch's
/// default (market-fetch does not set a UA at all).
#[test]
fn live_fetcher_sends_the_fixed_composite_user_agent() {
    assert_eq!(
        USER_AGENT, "curl/8.5.0 nullclaw/1.0",
        "must port the FIXED literal from fred_fetch.py:32"
    );
    let headers = fred_request_headers();
    let ua = headers
        .iter()
        .find(|(k, _)| *k == "User-Agent")
        .map(|(_, v)| *v)
        .expect("User-Agent header must be present");
    assert_eq!(ua, "curl/8.5.0 nullclaw/1.0");
    assert_ne!(
        ua, "nullclaw/1.0",
        "bare nullclaw/1.0 is refused by FRED (hangs)"
    );
    // Leading token is what FRED allowlists.
    assert!(
        ua.starts_with("curl/"),
        "FRED matches the leading token: {ua}"
    );
}
