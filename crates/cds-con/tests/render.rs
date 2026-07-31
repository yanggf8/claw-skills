//! Render-layer tests for cds-con Task 2.
//!
//! The golden message in the plan is the oracle. Every fixed rule (order,
//! windows, precision, units, provenance, freshness, missing series, missing
//! kind, monthly min-latest) has its own named test so a mutation gate can
//! point at a specific failure.

use cds_con::render::{
    analyze, format_message, render_lines, Frequency, SeriesInput, SeriesLine, WindowPct,
    BAA_AAA_KEY,
};
use credit_store::{Observation, SeriesKind, SeriesSpec};

// ── helpers ──────────────────────────────────────────────────────────────

fn spec(key: &str, kind: SeriesKind) -> SeriesSpec {
    SeriesSpec {
        key: key.into(),
        series_id: format!("FRED_{key}"),
        label: key.into(),
        kind: Some(kind),
    }
}

fn spec_no_kind(key: &str) -> SeriesSpec {
    SeriesSpec {
        key: key.into(),
        series_id: format!("FRED_{key}"),
        label: key.into(),
        kind: None,
    }
}

fn obs(rows: &[(&str, f64)]) -> Vec<Observation> {
    rows.iter()
        .map(|(d, v)| Observation {
            date: (*d).into(),
            value: *v,
        })
        .collect()
}

fn input(key: &str, kind: SeriesKind, freq: Frequency, rows: Vec<Observation>) -> SeriesInput {
    SeriesInput {
        spec: spec(key, kind),
        rows,
        frequency: freq,
    }
}

/// Plan golden numbers as precomputed SeriesLine rows (layout oracle).
fn golden_lines() -> Vec<SeriesLine> {
    vec![
        SeriesLine {
            key: BAA_AAA_KEY.into(),
            kind: SeriesKind::Spread,
            value: Some(0.48),
            windows: vec![
                WindowPct {
                    label: "1y",
                    pctile: 0.0,
                },
                WindowPct {
                    label: "10y",
                    pctile: 0.0,
                },
                WindowPct {
                    label: "全庫",
                    pctile: 3.7,
                },
            ],
            coverage_start: Some("1919-01-01".into()),
            latest: Some("2026-06-01".into()),
            frequency: Frequency::Monthly,
            config_order: 0,
        },
        SeriesLine {
            key: "baa10y".into(),
            kind: SeriesKind::Spread,
            value: Some(1.59),
            windows: vec![
                WindowPct {
                    label: "1y",
                    pctile: 14.8,
                },
                WindowPct {
                    label: "10y",
                    pctile: 10.2,
                },
                WindowPct {
                    label: "全庫",
                    pctile: 11.2,
                },
            ],
            coverage_start: Some("1986-01-02".into()),
            latest: Some("2026-07-24".into()),
            frequency: Frequency::Daily,
            config_order: 1,
        },
        SeriesLine {
            key: "hy_oas".into(),
            kind: SeriesKind::Spread,
            value: Some(2.79),
            windows: vec![
                WindowPct {
                    label: "1y",
                    pctile: 30.2,
                },
                WindowPct {
                    label: "全庫",
                    pctile: 18.1,
                },
            ],
            coverage_start: Some("2023-07-28".into()),
            latest: Some("2026-07-24".into()),
            frequency: Frequency::Daily,
            config_order: 2,
        },
        SeriesLine {
            key: "ig_oas".into(),
            kind: SeriesKind::Spread,
            value: Some(0.80),
            windows: vec![
                WindowPct {
                    label: "1y",
                    pctile: 60.0,
                },
                WindowPct {
                    label: "全庫",
                    pctile: 22.1,
                },
            ],
            coverage_start: Some("2023-07-28".into()),
            latest: Some("2026-07-24".into()),
            frequency: Frequency::Daily,
            config_order: 3,
        },
        SeriesLine {
            key: "aaa".into(),
            kind: SeriesKind::Yield,
            value: Some(5.52),
            windows: vec![
                WindowPct {
                    label: "1y",
                    pctile: 84.6,
                },
                WindowPct {
                    label: "10y",
                    pctile: 96.7,
                },
                WindowPct {
                    label: "全庫",
                    pctile: 61.6,
                },
            ],
            coverage_start: Some("1919-01-01".into()),
            latest: Some("2026-06-01".into()),
            frequency: Frequency::Monthly,
            config_order: 4,
        },
        SeriesLine {
            key: "baa".into(),
            kind: SeriesKind::Yield,
            value: Some(6.00),
            windows: vec![
                WindowPct {
                    label: "1y",
                    pctile: 46.2,
                },
                WindowPct {
                    label: "10y",
                    pctile: 86.0,
                },
                WindowPct {
                    label: "全庫",
                    pctile: 46.7,
                },
            ],
            coverage_start: Some("1919-01-01".into()),
            latest: Some("2026-06-01".into()),
            frequency: Frequency::Monthly,
            config_order: 5,
        },
        SeriesLine {
            key: "hy_yield".into(),
            kind: SeriesKind::Yield,
            value: Some(7.19),
            windows: vec![
                WindowPct {
                    label: "1y",
                    pctile: 97.0,
                },
                WindowPct {
                    label: "全庫",
                    pctile: 93.2,
                },
            ],
            coverage_start: Some("2023-07-28".into()),
            latest: Some("2026-07-24".into()),
            frequency: Frequency::Daily,
            config_order: 6,
        },
        SeriesLine {
            key: "ig_yield".into(),
            kind: SeriesKind::Yield,
            value: Some(5.43),
            windows: vec![
                WindowPct {
                    label: "1y",
                    pctile: 99.2,
                },
                WindowPct {
                    label: "全庫",
                    pctile: 96.4,
                },
            ],
            coverage_start: Some("2023-07-28".into()),
            latest: Some("2026-07-24".into()),
            frequency: Frequency::Daily,
            config_order: 7,
        },
        SeriesLine {
            key: "ccc_yield".into(),
            kind: SeriesKind::Yield,
            value: Some(14.28),
            windows: vec![
                WindowPct {
                    label: "1y",
                    pctile: 99.6,
                },
                WindowPct {
                    label: "全庫",
                    pctile: 93.5,
                },
            ],
            coverage_start: Some("2023-07-28".into()),
            latest: Some("2026-07-24".into()),
            frequency: Frequency::Daily,
            config_order: 8,
        },
    ]
}

/// Golden message: plan's 2026-07-31 live values under a consistent layout rule
/// (`key` w9, `value` w5, windows field w34). The plan document's hand-typed
/// sample pads some two-window / three-window lines one space wider (35) than
/// others (34); that inconsistency is not part of the shape. Every number,
/// label, order, unit rule and literal matches the plan.
const GOLDEN: &str = "\
💾 CDS-CON 信用利差

利差(已扣無風險利率)
  baa−aaa   0.48   1y p0.0 · 10y p0.0 · 全庫 p3.7      1919-01-01→ monthly
  baa10y    1.59   1y p14.8 · 10y p10.2 · 全庫 p11.2   1986-01-02→ daily
  hy_oas    2.79   1y p30.2 · 全庫 p18.1               2023-07-28→ daily
  ig_oas    0.80   1y p60.0 · 全庫 p22.1               2023-07-28→ daily

殖利率(含無風險利率 — 水位高低多半反映利率,不是信用壓力)
  aaa       5.52   1y p84.6 · 10y p96.7 · 全庫 p61.6   1919-01-01→ monthly
  baa       6.00   1y p46.2 · 10y p86.0 · 全庫 p46.7   1919-01-01→ monthly
  hy_yield  7.19   1y p97.0 · 全庫 p93.2               2023-07-28→ daily
  ig_yield  5.43   1y p99.2 · 全庫 p96.4               2023-07-28→ daily
  ccc_yield 14.28  1y p99.6 · 全庫 p93.5               2023-07-28→ daily

資料:daily 至 2026-07-24(7 天前) · monthly 至 2026-06-01
SIGNAL-ONLY:百分位是窗口內的排名,窗口會翻轉結論。";

// ── exact shape ──────────────────────────────────────────────────────────

#[test]
fn golden_message_matches_plan_exactly() {
    let rendered = render_lines(&golden_lines(), "2026-07-31");
    assert_eq!(
        rendered, GOLDEN,
        "rendered message must match the plan golden byte-for-byte\n--- rendered ---\n{rendered}\n--- golden ---\n{GOLDEN}"
    );
}

// ── order ────────────────────────────────────────────────────────────────

#[test]
fn order_spreads_before_yields_longest_coverage_first() {
    // Config order deliberately puts short-coverage first and yields first;
    // the renderer must reorder: spreads block, then longest coverage first.
    let series = vec![
        input(
            "hy_oas",
            SeriesKind::Spread,
            Frequency::Daily,
            obs(&[("2023-07-28", 2.0), ("2024-07-28", 2.5), ("2025-07-28", 2.8)]),
        ),
        input(
            "baa10y",
            SeriesKind::Spread,
            Frequency::Daily,
            // Coverage back to 1986 — must lead hy_oas even though later in config.
            obs(&[("1986-01-02", 1.0), ("2025-07-28", 1.5), ("2026-07-24", 1.59)]),
        ),
        input(
            "ccc_yield",
            SeriesKind::Yield,
            Frequency::Daily,
            obs(&[("2023-07-28", 10.0), ("2026-07-24", 14.0)]),
        ),
        input(
            "aaa",
            SeriesKind::Yield,
            Frequency::Monthly,
            obs(&[("1919-01-01", 5.0), ("2026-06-01", 5.5)]),
        ),
        // baa + aaa both present → derived baa−aaa (1919) leads the spread block.
        input(
            "baa",
            SeriesKind::Yield,
            Frequency::Monthly,
            obs(&[("1919-01-01", 5.5), ("2026-06-01", 6.0)]),
        ),
    ];

    let lines = analyze(&series).expect("kinds present");
    let keys: Vec<&str> = lines.iter().map(|l| l.key.as_str()).collect();

    // Spreads first (baa−aaa 1919, baa10y 1986, hy_oas 2023), then yields
    // (aaa 1919 before ccc_yield 2023; baa same start as aaa → config order).
    assert_eq!(
        keys[0], BAA_AAA_KEY,
        "derived baa−aaa (1919) must lead the spread block; got {keys:?}"
    );
    let spread_keys: Vec<&str> = lines
        .iter()
        .filter(|l| l.kind == SeriesKind::Spread)
        .map(|l| l.key.as_str())
        .collect();
    assert_eq!(
        spread_keys,
        vec![BAA_AAA_KEY, "baa10y", "hy_oas"],
        "spreads longest-coverage-first"
    );

    let yield_keys: Vec<&str> = lines
        .iter()
        .filter(|l| l.kind == SeriesKind::Yield)
        .map(|l| l.key.as_str())
        .collect();
    // aaa and baa share 1919; config order was ccc, aaa, baa → among 1919:
    // aaa (index of aaa in input) before baa. ccc_yield 2023 last.
    assert_eq!(
        yield_keys[0], "aaa",
        "longest yield coverage first; got {yield_keys:?}"
    );
    assert_eq!(yield_keys.last().copied(), Some("ccc_yield"));

    // All spreads appear before all yields in the flat list.
    let last_spread = lines.iter().rposition(|l| l.kind == SeriesKind::Spread).unwrap();
    let first_yield = lines.iter().position(|l| l.kind == SeriesKind::Yield).unwrap();
    assert!(
        last_spread < first_yield,
        "every spread must precede every yield; keys={keys:?}"
    );
}

// ── windows ──────────────────────────────────────────────────────────────

#[test]
fn unreachable_window_is_omitted_not_printed_as_insufficient() {
    // ~3y of daily data: 1y ok, 10y insufficient → omit 10y entirely.
    let rows = obs(&[
        ("2023-07-28", 1.0),
        ("2024-07-28", 1.5),
        ("2025-07-28", 2.0),
        ("2026-07-24", 2.5),
    ]);
    let series = vec![input(
        "hy_oas",
        SeriesKind::Spread,
        Frequency::Daily,
        rows,
    )];
    let lines = analyze(&series).unwrap();
    assert_eq!(lines.len(), 1);
    let labels: Vec<&str> = lines[0].windows.iter().map(|w| w.label).collect();
    assert_eq!(
        labels,
        vec!["1y", "全庫"],
        "10y must be omitted for short coverage, not listed; got {labels:?}"
    );

    let msg = render_lines(&lines, "2026-07-31");
    assert!(
        !msg.contains("insufficient-coverage"),
        "must never print insufficient-coverage in the message: {msg}"
    );
    assert!(
        !msg.contains("10y"),
        "unreachable 10y window must not appear: {msg}"
    );
    assert!(msg.contains("1y p"), "1y must still appear: {msg}");
    assert!(msg.contains("全庫 p"), "全庫 must still appear: {msg}");
    // Omission is legible because coverage start is on the same line.
    assert!(
        msg.contains("2023-07-28→ daily"),
        "coverage start must remain on the line: {msg}"
    );
}

#[test]
fn long_coverage_series_shows_all_three_windows() {
    // Coverage from 1919 supports 1y, 10y and 全庫 — including derived baa−aaa.
    let aaa = obs(&[
        ("1919-01-01", 5.0),
        ("2016-01-01", 4.0),
        ("2020-01-01", 3.5),
        ("2025-01-01", 5.0),
        ("2026-06-01", 5.5),
    ]);
    let baa = obs(&[
        ("1919-01-01", 5.5),
        ("2016-01-01", 4.8),
        ("2020-01-01", 4.0),
        ("2025-01-01", 5.8),
        ("2026-06-01", 6.0),
    ]);
    let series = vec![
        input("aaa", SeriesKind::Yield, Frequency::Monthly, aaa),
        input("baa", SeriesKind::Yield, Frequency::Monthly, baa),
    ];
    let lines = analyze(&series).unwrap();
    let derived = lines.iter().find(|l| l.key == BAA_AAA_KEY).expect("derived");
    let labels: Vec<&str> = derived.windows.iter().map(|w| w.label).collect();
    assert_eq!(
        labels,
        vec!["1y", "10y", "全庫"],
        "baa−aaa reaches 1919 so it shows all three windows; got {labels:?}"
    );
}

// ── precision ────────────────────────────────────────────────────────────

#[test]
fn precision_value_two_decimals_percentile_one() {
    let line = SeriesLine {
        key: "ig_oas".into(),
        kind: SeriesKind::Spread,
        value: Some(0.8),
        windows: vec![
            WindowPct {
                label: "1y",
                pctile: 60.0,
            },
            WindowPct {
                label: "全庫",
                pctile: 22.14,
            },
        ],
        coverage_start: Some("2023-07-28".into()),
        latest: Some("2026-07-24".into()),
        frequency: Frequency::Daily,
        config_order: 0,
    };
    let msg = render_lines(&[line], "2026-07-31");
    assert!(
        msg.contains("0.80"),
        "value must be two decimal places: {msg}"
    );
    assert!(
        msg.contains("p60.0"),
        "percentile must be one decimal place: {msg}"
    );
    assert!(
        msg.contains("p22.1"),
        "percentile rounds to one decimal: {msg}"
    );
    // Not unrounded / three-decimal forms.
    assert!(!msg.contains("0.800"), "{msg}");
    assert!(!msg.contains("p22.14"), "{msg}");
}

// ── units ────────────────────────────────────────────────────────────────

#[test]
fn no_percent_units_printed() {
    let msg = render_lines(&golden_lines(), "2026-07-31");
    assert!(
        !msg.contains('%'),
        "no % unit — mixing % with p12.7 invites reading a percentile as a rate: {msg}"
    );
}

// ── provenance ───────────────────────────────────────────────────────────

#[test]
fn provenance_is_coverage_and_frequency_not_fred_id() {
    let series = vec![input(
        "baa10y",
        SeriesKind::Spread,
        Frequency::Daily,
        obs(&[("1986-01-02", 1.0), ("2026-07-24", 1.59)]),
    )];
    // series_id is FRED_baa10y via helper — must not appear.
    let msg = format_message(&series, "2026-07-31").unwrap();
    assert!(
        msg.contains("1986-01-02→ daily"),
        "coverage start and frequency required: {msg}"
    );
    assert!(
        !msg.contains("FRED_"),
        "FRED id must not appear in the daily message: {msg}"
    );
    assert!(
        !msg.contains("BAA10Y") && !msg.contains("BAML"),
        "raw FRED series ids are for `cds show`, not the message: {msg}"
    );
}

// ── freshness ────────────────────────────────────────────────────────────

#[test]
fn freshness_line_shows_age_without_judgment() {
    let msg = render_lines(&golden_lines(), "2026-07-31");
    assert!(
        msg.contains("資料:daily 至 2026-07-24(7 天前)"),
        "daily latest and age-in-days required: {msg}"
    );
    assert!(
        msg.contains("monthly 至 2026-06-01"),
        "monthly latest required: {msg}"
    );
    // No threshold, colour, or adjective about staleness.
    for banned in ["stale", "過期", "新鮮", "陳舊", "⚠️", "degraded", "新鮮度"] {
        assert!(
            !msg.contains(banned),
            "freshness must not judge the age (found '{banned}'): {msg}"
        );
    }
}

#[test]
fn monthly_freshness_uses_minimum_latest_not_maximum() {
    // baa latest 2026-07-01, aaa latest 2026-06-01 → derived join stops at
    // 2026-06-01. The 資料 line must report monthly 至 2026-06-01 (the min),
    // never 2026-07-01 (the max of the inputs).
    let aaa = obs(&[
        ("2020-01-01", 5.0),
        ("2026-05-01", 5.4),
        ("2026-06-01", 5.5),
        // aaa has not published July yet
    ]);
    let baa = obs(&[
        ("2020-01-01", 5.5),
        ("2026-05-01", 5.9),
        ("2026-06-01", 6.0),
        ("2026-07-01", 6.1), // baa landed first
    ]);
    let series = vec![
        input("aaa", SeriesKind::Yield, Frequency::Monthly, aaa),
        input("baa", SeriesKind::Yield, Frequency::Monthly, baa),
    ];
    let msg = format_message(&series, "2026-07-31").unwrap();
    assert!(
        msg.contains("monthly 至 2026-06-01"),
        "monthly freshness must be the MIN latest (derived lags at 2026-06-01): {msg}"
    );
    assert!(
        !msg.contains("monthly 至 2026-07-01"),
        "must not advertise max monthly latest the headline number does not have: {msg}"
    );
}

// ── missing series ───────────────────────────────────────────────────────

#[test]
fn missing_series_renders_na_and_is_named_in_freshness() {
    let series = vec![
        input(
            "baa10y",
            SeriesKind::Spread,
            Frequency::Daily,
            obs(&[("1986-01-02", 1.0), ("2026-07-24", 1.59)]),
        ),
        input(
            "hy_oas",
            SeriesKind::Spread,
            Frequency::Daily,
            vec![], // missing
        ),
        input(
            "aaa",
            SeriesKind::Yield,
            Frequency::Monthly,
            obs(&[("1919-01-01", 5.0), ("2026-06-01", 5.5)]),
        ),
    ];
    let msg = format_message(&series, "2026-07-31").unwrap();
    assert!(
        msg.contains("hy_oas") && msg.contains("n/a"),
        "missing series must render as n/a, not vanish: {msg}"
    );
    // Row still present with frequency (coverage line structure intact).
    assert!(
        msg.lines().any(|l| l.contains("hy_oas") && l.contains("n/a") && l.contains("daily")),
        "n/a row keeps frequency on the line: {msg}"
    );
    assert!(
        msg.contains("缺 hy_oas") || msg.contains("缺") && msg.contains("hy_oas"),
        "資料 line must name the missing series: {msg}"
    );
}

// ── missing kind ─────────────────────────────────────────────────────────

#[test]
fn missing_kind_fails_loudly_not_defaulted_to_yield() {
    let series = vec![SeriesInput {
        spec: spec_no_kind("hy_oas"),
        rows: obs(&[("2023-07-28", 2.0), ("2026-07-24", 2.5)]),
        frequency: Frequency::Daily,
    }];
    let err = format_message(&series, "2026-07-31").expect_err("missing kind must err");
    assert!(
        err.message.contains("hy_oas") && err.message.contains("kind"),
        "error must name the series and the missing kind: {}",
        err.message
    );
    // Must not silently render under the yield (or spread) block.
    // If someone defaults to Yield, format_message would return Ok.
}

// ── structural block split ───────────────────────────────────────────────

#[test]
fn spread_and_yield_blocks_are_separate_with_meaning_labels() {
    let msg = render_lines(&golden_lines(), "2026-07-31");
    let spread_hdr = msg.find("利差(已扣無風險利率)").expect("spread header");
    let yield_hdr = msg
        .find("殖利率(含無風險利率 — 水位高低多半反映利率,不是信用壓力)")
        .expect("yield header");
    assert!(
        spread_hdr < yield_hdr,
        "spread block must precede yield block"
    );
    // Keys land under the right header.
    let baa10y = msg.find("baa10y").unwrap();
    let aaa = msg.find("\n  aaa ").unwrap_or_else(|| msg.find("aaa").unwrap());
    assert!(baa10y > spread_hdr && baa10y < yield_hdr, "baa10y under spreads");
    assert!(aaa > yield_hdr, "aaa under yields");
}

// ── SIGNAL-ONLY closer ───────────────────────────────────────────────────

#[test]
fn closes_with_signal_only_and_has_no_status_line() {
    let msg = render_lines(&golden_lines(), "2026-07-31");
    assert!(
        msg.contains("SIGNAL-ONLY:百分位是窗口內的排名,窗口會翻轉結論。"),
        "{msg}"
    );
    assert!(
        !msg.contains("狀態：") && !msg.contains("狀態:"),
        "cds-con deliberately has no 狀態 line: {msg}"
    );
}
