//! Rendering + config tests. Expectations are quoted from
//! inflation-con/scripts/run.py — not from the plan's prose summary.

use inflation_con::analysis::{Details, Status};
use inflation_con::config::{load_config, Config, DEFAULT_SERIES, VALID_STANCES};
use inflation_con::render::{fmt_num, fmt_pct, format_message, record_line};

fn details_full() -> Details {
    Details {
        core_pce_day: "2026-05-01".into(),
        pce3: Some(4.0),
        pce6: Some(3.5),
        cpi3: Some(3.2),
        cpi6: Some(3.1),
        breakeven: Some(2.60),
        breakeven_day: Some("2026-06-15".into()),
        breakeven_rising: Some(true),
        policy_stance: "restrictive".into(),
        core_pce_obs: 12,
        reasons: vec!["core PCE 3-mo & 6-mo annualized both >= 3.5%".into()],
    }
}

fn details_insufficient(obs: usize) -> Details {
    Details {
        core_pce_day: String::new(),
        pce3: None,
        pce6: None,
        cpi3: None,
        cpi6: None,
        breakeven: None,
        breakeven_day: None,
        breakeven_rising: None,
        policy_stance: String::new(),
        core_pce_obs: obs,
        reasons: vec![
            "fewer than 7 monthly core-PCE observations or missing latest core-PCE/CPI".into(),
        ],
    }
}

fn cfg() -> Config {
    Config {
        series: DEFAULT_SERIES
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect(),
        policy_stance: "unclear".into(),
    }
}

// ---- fmt_pct / fmt_num (run.py:242-250) ------------------------------------

#[test]
fn fmt_pct_none_is_n_a() {
    // run.py:243-244
    assert_eq!(fmt_pct(None), "n/a");
}

#[test]
fn fmt_pct_positive_carries_plus_and_two_decimals() {
    // run.py:245-246  sign = "+" if v >= 0 else ""; f"{sign}{v:.2f}%"
    assert_eq!(fmt_pct(Some(4.0)), "+4.00%");
    assert_eq!(fmt_pct(Some(0.0)), "+0.00%");
}

#[test]
fn fmt_pct_negative_has_no_extra_plus() {
    // run.py:245-246  negative keeps the numeric minus only
    assert_eq!(fmt_pct(Some(-1.5)), "-1.50%");
}

#[test]
fn fmt_num_none_is_n_a_else_two_decimals() {
    // run.py:249-250
    assert_eq!(fmt_num(None), "n/a");
    assert_eq!(fmt_num(Some(2.6)), "2.60");
}

// ---- skill status vs classification (run.py:253-258, 285) -----------------

#[test]
fn a_warning_makes_the_skill_status_degraded_but_not_the_classification() {
    // run.py:255-258  skill_status = "ok"; if warning: skill_status = "degraded"
    let (_, s) = format_message(Status::Ok, &details_full(), &cfg(), Some("T10YIE: no rows"));
    assert_eq!(s, "degraded");
    let (_, s) = format_message(Status::Ok, &details_full(), &cfg(), None);
    assert_eq!(s, "ok");
}

#[test]
fn a_red_classification_without_a_warning_is_still_skill_status_ok() {
    // run.py:255-258 — skill status is independent of classification.
    // RED is a market signal; reporting it as degraded would make the
    // scheduler retry a perfectly successful run.
    let (_, s) = format_message(Status::Red, &details_full(), &cfg(), None);
    assert_eq!(s, "ok");
    let (m, _) = format_message(Status::Red, &details_full(), &cfg(), None);
    assert!(m.contains("狀態：RED"), "classification still renders: {m}");
}

// ---- three-valued breakeven_rising (run.py:264-265, 269) ------------------

#[test]
fn breakeven_rising_display_is_three_valued() {
    // run.py:264-265
    // rising_str = "rising" if rising is True else "flat/down" if rising is False else "n/a"
    let mut d = details_full();

    d.breakeven_rising = Some(true);
    let (m, _) = format_message(Status::Red, &d, &cfg(), None);
    assert!(
        m.contains("10Y breakeven：2.60 (2026-06-15, rising)"),
        "rising branch: {m}"
    );

    d.breakeven_rising = Some(false);
    let (m, _) = format_message(Status::Red, &d, &cfg(), None);
    assert!(
        m.contains("10Y breakeven：2.60 (2026-06-15, flat/down)"),
        "flat/down branch: {m}"
    );

    d.breakeven_rising = None;
    let (m, _) = format_message(Status::Red, &d, &cfg(), None);
    assert!(
        m.contains("10Y breakeven：2.60 (2026-06-15, n/a)"),
        "n/a branch must not collapse into flat/down: {m}"
    );
    assert!(
        !m.contains("flat/down"),
        "None must not render as flat/down: {m}"
    );
}

// ---- INSUFFICIENT_DATA skips indicator block (run.py:261-274) -------------

#[test]
fn insufficient_data_renders_obs_count_and_skips_indicators() {
    // run.py:261-262 only; the else branch (263-274) is skipped.
    let d = details_insufficient(5);
    let (m, _) = format_message(Status::InsufficientData, &d, &cfg(), None);
    assert!(
        m.contains("core PCE obs: 5 / 7 needed"),
        "exact obs line from run.py:262: {m}"
    );
    // Indicator-block needles from run.py:267-270 only (not the manual-check
    // trailer, which also mentions "FOMC 立場" with full-width parens).
    assert!(
        !m.contains("核心PCE"),
        "indicator block must be skipped: {m}"
    );
    assert!(
        !m.contains("核心CPI"),
        "indicator block must be skipped: {m}"
    );
    assert!(
        !m.contains("10Y breakeven"),
        "indicator block must be skipped: {m}"
    );
    assert!(
        !m.contains("FOMC 立場 (manual)"),
        "indicator FOMC line must be skipped: {m}"
    );
    assert!(
        !m.contains("依據："),
        "reasons block is inside the else — must be skipped: {m}"
    );
    // But the manual-check + SIGNAL-ONLY trailer still render (run.py:276-283).
    assert!(m.contains("人工檢查（不入演算法）："), "{m}");
    assert!(m.contains("SIGNAL-ONLY"), "{m}");
}

// ---- trailing lines (run.py:282-283) --------------------------------------

#[test]
fn message_carries_signal_only_and_red_review_lines() {
    // run.py:282-283 — chipcon has only SIGNAL-ONLY; this skill has both.
    let (m, _) = format_message(Status::Red, &details_full(), &cfg(), None);
    assert!(
        m.contains("SIGNAL-ONLY：這是通膨『確認證據』分級，不是交易指令。"),
        "exact SIGNAL-ONLY line: {m}"
    );
    assert!(
        m.contains("RED = 進入 review（是否加通膨對沖？IEF gate？），由人決定並記 decision add。"),
        "exact RED review line: {m}"
    );
}

#[test]
fn message_prefix_and_status_line_match_python() {
    // run.py:254, 259
    let (m, _) = format_message(Status::Watch, &details_full(), &cfg(), None);
    assert!(m.starts_with("📈 INFLATION-CON\n"), "{m}");
    assert!(m.contains("狀態：WATCH"), "{m}");
}

#[test]
fn warning_line_is_prefixed_exactly() {
    // run.py:257
    let (m, _) = format_message(
        Status::Ok,
        &details_full(),
        &cfg(),
        Some("fetch DGS10: timeout"),
    );
    assert!(m.contains("⚠ 這份不完整:fetch DGS10: timeout"), "{m}");
}

#[test]
fn indicator_block_and_reasons_match_python_shapes() {
    // run.py:266-274
    let (m, _) = format_message(Status::Red, &details_full(), &cfg(), None);
    assert!(
        m.contains("核心PCE (2026-05-01)：3mo +4.00% / 6mo +3.50% 年化"),
        "{m}"
    );
    assert!(
        m.contains("核心CPI：3mo +3.20% / 6mo +3.10% 年化"),
        "{m}"
    );
    assert!(
        m.contains("10Y breakeven：2.60 (2026-06-15, rising)"),
        "{m}"
    );
    assert!(m.contains("FOMC 立場 (manual)：restrictive"), "{m}");
    assert!(m.contains("依據："), "{m}");
    assert!(
        m.contains("- core PCE 3-mo & 6-mo annualized both >= 3.5%"),
        "{m}"
    );
}

// ---- record_line (run.py:287-297) — timestamp is a parameter -------------

#[test]
fn record_line_takes_the_clock_as_a_parameter() {
    // Structural change vs Python: now is injected so the line is unit-testable.
    // Python (run.py:288) calls datetime.now(...); Rust takes `now: &str`.
    let l = record_line(
        Status::Red,
        &details_full(),
        None,
        "2026-07-30 06:00:00 CST",
    );
    assert!(
        l.starts_with("2026-07-30 06:00:00 CST INFLATION-CON RED "),
        "{l}"
    );
}

#[test]
fn record_line_normal_shape_carries_indicators_and_warning_dash() {
    // run.py:291-297  normal shape + warning={warning or '-'}
    let l = record_line(
        Status::Red,
        &details_full(),
        None,
        "2026-07-30 06:00:00 CST",
    );
    assert_eq!(
        l,
        "2026-07-30 06:00:00 CST INFLATION-CON RED \
         pce3=+4.00% pce6=+3.50% cpi3=+3.20% cpi6=+3.10% \
         be=2.60 stance=restrictive warning=-"
    );
}

#[test]
fn record_line_insufficient_data_shape_carries_obs() {
    // run.py:289-290
    let d = details_insufficient(5);
    let l = record_line(
        Status::InsufficientData,
        &d,
        Some("boom"),
        "2026-07-30 06:00:00 CST",
    );
    assert_eq!(
        l,
        "2026-07-30 06:00:00 CST INFLATION-CON INSUFFICIENT_DATA obs=5 warning=boom"
    );
    assert!(
        !l.contains("pce3="),
        "indicator fields must be omitted: {l}"
    );
}

#[test]
fn record_line_insufficient_data_warning_dash_when_none() {
    // run.py:290  warning={warning or '-'}
    let d = details_insufficient(3);
    let l = record_line(
        Status::InsufficientData,
        &d,
        None,
        "2026-07-30 06:00:00 CST",
    );
    assert!(l.ends_with("warning=-"), "no warning renders as a dash: {l}");
}

// ---- Config / DEFAULT_SERIES / load_config (run.py:55-88) -----------------

#[test]
fn default_series_is_document_order_not_alphabetical() {
    // run.py:55-63 insertion order. Alphabetical would start with breakeven_10y.
    let keys: Vec<&str> = DEFAULT_SERIES.iter().map(|(k, _)| *k).collect();
    assert_eq!(
        keys,
        vec![
            "core_pce",
            "core_cpi",
            "headline_pce",
            "headline_cpi",
            "breakeven_10y",
            "real_yield_10y",
            "nominal_10y",
        ]
    );
    assert_ne!(
        keys[0], "breakeven_10y",
        "alphabetical BTreeMap order starts with breakeven_10y"
    );
}

#[test]
fn config_series_order_drives_joined_warning_text() {
    // Order test 1 of 2. Pins: a given ordered Config produces warnings in
    // that order. fetch_all (run.py:227) iterates series_map.items() and joins
    // with "; ". A BTreeMap field would alphabetize and reorder the join.
    // Non-alphabetical hand order: core_pce, nominal_10y, headline_cpi.
    // Alpha would be: core_pce, headline_cpi, nominal_10y.
    let cfg = Config {
        series: vec![
            ("core_pce".into(), "PCEPILFE".into()),
            ("nominal_10y".into(), "DGS10".into()),
            ("headline_cpi".into(), "CPIAUCSL".into()),
        ],
        policy_stance: "unclear".into(),
    };
    // Same join shape as fetch_all's empty-result warnings (run.py:230-231, 237).
    let joined: String = cfg
        .series
        .iter()
        .map(|(_, id)| format!("{id}: no rows"))
        .collect::<Vec<_>>()
        .join("; ");
    assert_eq!(
        joined, "PCEPILFE: no rows; DGS10: no rows; CPIAUCSL: no rows",
        "config order, not alphabetical"
    );
    // Alphabetical would put CPIAUCSL before DGS10.
    let alpha = "PCEPILFE: no rows; CPIAUCSL: no rows; DGS10: no rows";
    assert_ne!(joined, alpha);
}

#[test]
fn load_config_preserves_document_order_from_the_file() {
    // Order test 2 of 2. The test above pins "given an ordered Config,
    // iteration follows it". It does NOT pin that load_config PRODUCES an
    // ordered Config from the file. Python merges DEFAULT_SERIES then
    // update()s file keys — extras append in file insertion order
    // (run.py:82-85). That survives only because Cargo.toml enables
    // serde_json's preserve_order. Without it, Map is BTreeMap-backed and
    // extras would arrive alphabetically.
    let dir = std::env::temp_dir().join(format!(
        "inflation-con-cfg-order-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.json");
    // Extra keys not in DEFAULT_SERIES, in non-alphabetical file order.
    std::fs::write(
        &path,
        r#"{"series":{"zzz_extra":"ZZZ","aaa_extra":"AAA"},"policy_stance":"unclear"}"#,
    )
    .unwrap();
    let cfg = load_config(&path);
    let keys: Vec<&str> = cfg.series.iter().map(|(k, _)| k.as_str()).collect();
    // Defaults first (document order), then extras in FILE order (zzz before aaa).
    let zzz = keys.iter().position(|k| *k == "zzz_extra").expect("zzz_extra");
    let aaa = keys.iter().position(|k| *k == "aaa_extra").expect("aaa_extra");
    assert!(
        zzz < aaa,
        "file insertion order, not alphabetical: {keys:?}"
    );
    // Defaults still lead and stay in DEFAULT_SERIES order.
    assert_eq!(&keys[..7], &DEFAULT_SERIES.iter().map(|(k, _)| *k).collect::<Vec<_>>()[..]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn load_config_missing_path_returns_defaults() {
    // run.py:78-79
    let missing = std::env::temp_dir().join(format!(
        "inflation-con-missing-{}/nope.json",
        std::process::id()
    ));
    let cfg = load_config(&missing);
    assert_eq!(cfg.policy_stance, "unclear");
    assert_eq!(cfg.series.len(), DEFAULT_SERIES.len());
    for ((k, v), (dk, dv)) in cfg.series.iter().zip(DEFAULT_SERIES.iter()) {
        assert_eq!(k, dk);
        assert_eq!(v, *dv);
    }
}

#[test]
fn load_config_normalizes_stance_to_lowercase() {
    // run.py:86-87
    let dir = std::env::temp_dir().join(format!(
        "inflation-con-cfg-stance-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.json");
    std::fs::write(&path, r#"{"policy_stance":"RESTRICTIVE"}"#).unwrap();
    assert_eq!(load_config(&path).policy_stance, "restrictive");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn load_config_invalid_stance_falls_back_to_unclear() {
    // run.py:87  stance if stance in VALID_STANCES else "unclear"
    let dir = std::env::temp_dir().join(format!(
        "inflation-con-cfg-badstance-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.json");
    std::fs::write(&path, r#"{"policy_stance":"hawkish"}"#).unwrap();
    assert_eq!(load_config(&path).policy_stance, "unclear");
    assert!(VALID_STANCES.contains(&"restrictive"));
    assert!(VALID_STANCES.contains(&"neutral"));
    assert!(VALID_STANCES.contains(&"easing"));
    assert!(VALID_STANCES.contains(&"unclear"));
    assert!(!VALID_STANCES.contains(&"hawkish"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn load_config_merges_default_series_with_partial_override() {
    // run.py:82-85
    let dir = std::env::temp_dir().join(format!(
        "inflation-con-cfg-partial-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.json");
    std::fs::write(
        &path,
        r#"{"policy_stance":"neutral","series":{"core_pce":"CUSTOM_PCE"}}"#,
    )
    .unwrap();
    let cfg = load_config(&path);
    assert_eq!(cfg.policy_stance, "neutral");
    let pce = cfg
        .series
        .iter()
        .find(|(k, _)| k == "core_pce")
        .expect("core_pce");
    assert_eq!(pce.1, "CUSTOM_PCE");
    let cpi = cfg
        .series
        .iter()
        .find(|(k, _)| k == "core_cpi")
        .expect("core_cpi");
    assert_eq!(cpi.1, "CPILFESL");
    assert_eq!(cfg.series.len(), DEFAULT_SERIES.len());
    let _ = std::fs::remove_dir_all(&dir);
}
