use chipcon::fetch::{update_state, yahoo_rows_or_empty};
use chipcon::analysis::Row;
use chipcon::config::Config;
use market_fetch::yahoo::FetchError;

fn cfg() -> Config {
    Config {
        symbols: vec![
            ("SMH".into(), "SMH".into()),
            ("QQQ".into(), "QQQ".into()),
            ("SOXX".into(), "SOXX".into()),
        ],
        position_label: String::new(),
        manual_events: vec![],
    }
}

fn rows(n: usize) -> Vec<Row> {
    (0..n).map(|i| Row { day: format!("2026-07-{:02}", i + 1), close: 100.0 + i as f64 }).collect()
}

#[test]
fn success_sorts_ascending_by_date() {
    let mut unsorted = rows(3);
    unsorted.reverse();
    let (state, warn) = update_state(&cfg(), &|_| Ok(unsorted.clone())).unwrap();
    assert!(warn.is_none());
    let smh = &state["SMH"];
    assert_eq!(smh[0].day, "2026-07-01");
    assert_eq!(smh[2].day, "2026-07-03");
}

#[test]
fn a_failing_primary_symbol_is_a_hard_error() {
    let e = update_state(&cfg(), &|_| Err(FetchError::Http("boom".into()))).unwrap_err();
    assert!(e.contains("SMH"), "{e}");
}

#[test]
fn an_empty_primary_symbol_is_a_hard_error() {
    let e = update_state(&cfg(), &|_| Ok(vec![])).unwrap_err();
    assert!(e.contains("SMH"), "{e}");
}

#[test]
fn a_failing_secondary_symbol_only_warns() {
    let (state, warn) = update_state(&cfg(), &|sym| {
        if sym == "SMH" { Ok(rows(30)) } else { Err(FetchError::Http("boom".into())) }
    }).unwrap();
    assert_eq!(state["SMH"].len(), 30);
    assert!(state["QQQ"].is_empty());
    let w = warn.expect("a secondary failure must warn");
    assert!(w.contains("yahoo fetch QQQ:"), "{w}");
}

#[test]
fn an_empty_secondary_symbol_warns_with_the_no_rows_wording() {
    let (_, warn) = update_state(&cfg(), &|sym| {
        if sym == "SMH" { Ok(rows(30)) } else { Ok(vec![]) }
    }).unwrap();
    let w = warn.expect("an empty secondary must warn");
    assert!(w.contains("yahoo QQQ: no rows"), "wording must match the Python: {w}");
}

#[test]
fn warnings_follow_config_order_not_alphabetical_order() {
    // Python iterates the symbols dict in insertion order: SMH, QQQ, SOXX.
    // A BTreeMap would give QQQ, SMH, SOXX. The difference only shows when
    // several warnings are joined — which happens on the fatal path, where the
    // joined text becomes the `CHIPCON failed: …` line nullclaw stores.
    let e = update_state(&cfg(), &|_| Ok(vec![])).unwrap_err();
    let smh = e.find("SMH").expect("SMH warning missing");
    let qqq = e.find("QQQ").expect("QQQ warning missing");
    let soxx = e.find("SOXX").expect("SOXX warning missing");
    assert!(smh < qqq && qqq < soxx, "config order, not alphabetical: {e}");
}

#[test]
fn load_config_preserves_document_order_from_the_file() {
    // The order test above pins "given an ordered Config, warnings follow it".
    // It does NOT pin that load_config PRODUCES an ordered Config, because the
    // cfg() helper builds one directly. Order survives only because Cargo.toml
    // enables serde_json's `preserve_order` feature — and a feature flag is the
    // easiest thing in a manifest to drop during a dependency cleanup. Measured
    // 2026-07-29: removing that feature left all 33 tests green. This test is
    // what makes the flag load-bearing instead of decorative.
    let dir = std::env::temp_dir().join(format!("chipcon-cfg-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.json");
    std::fs::write(&path, r#"{"symbols":{"SMH":"SMH","QQQ":"QQQ","SOXX":"SOXX"}}"#).unwrap();
    let cfg = chipcon::config::load_config(&path);
    let keys: Vec<&str> = cfg.symbols.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(keys, vec!["SMH", "QQQ", "SOXX"], "document order, not alphabetical");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn upstream_and_no_data_both_become_an_empty_series() {
    // Python's parser returns [] for chart.error AND for a missing result or
    // falsy closes. Both must land on the "no rows" warning, not the
    // "fetch failed" one, or the message text silently changes.
    assert!(yahoo_rows_or_empty(Err(FetchError::Upstream("Not Found".into()))).unwrap().is_empty());
    assert!(yahoo_rows_or_empty(Err(FetchError::NoData)).unwrap().is_empty());
}

#[test]
fn transport_and_parse_failures_stay_errors() {
    assert!(yahoo_rows_or_empty(Err(FetchError::Http("timeout".into()))).is_err());
    assert!(yahoo_rows_or_empty(Err(FetchError::Parse("bad json".into()))).is_err());
}
