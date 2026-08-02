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
            label: BAA_AAA_KEY.into(),
            kind: SeriesKind::Spread,
            value: Some(0.48),
            windows: vec![
                WindowPct {
                    label: "1年",
                    pctile: 0.0,
                },
                WindowPct {
                    label: "10年",
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
            label: "baa10y".into(),
            kind: SeriesKind::Spread,
            value: Some(1.59),
            windows: vec![
                WindowPct {
                    label: "1年",
                    pctile: 14.8,
                },
                WindowPct {
                    label: "10年",
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
            label: "hy_oas".into(),
            kind: SeriesKind::Spread,
            value: Some(2.79),
            windows: vec![
                WindowPct {
                    label: "1年",
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
            label: "ig_oas".into(),
            kind: SeriesKind::Spread,
            value: Some(0.80),
            windows: vec![
                WindowPct {
                    label: "1年",
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
            label: "aaa".into(),
            kind: SeriesKind::Yield,
            value: Some(5.52),
            windows: vec![
                WindowPct {
                    label: "1年",
                    pctile: 84.6,
                },
                WindowPct {
                    label: "10年",
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
            label: "baa".into(),
            kind: SeriesKind::Yield,
            value: Some(6.00),
            windows: vec![
                WindowPct {
                    label: "1年",
                    pctile: 46.2,
                },
                WindowPct {
                    label: "10年",
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
            label: "hy_yield".into(),
            kind: SeriesKind::Yield,
            value: Some(7.19),
            windows: vec![
                WindowPct {
                    label: "1年",
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
            label: "ig_yield".into(),
            kind: SeriesKind::Yield,
            value: Some(5.43),
            windows: vec![
                WindowPct {
                    label: "1年",
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
            label: "ccc_yield".into(),
            kind: SeriesKind::Yield,
            value: Some(14.28),
            windows: vec![
                WindowPct {
                    label: "1年",
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
const GOLDEN: &str = "💾 信用利差\n\n利差(已扣掉無風險利率 —— 這是信用風險本身的價格)\n  baa−aaa [baa−aaa]       0.48%   1年 p0 · 10年 p0 · 全庫 p3      自1919・月頻\n  baa10y [baa10y]         1.59%   1年 p14 · 10年 p10 · 全庫 p11   自1986・日頻\n  hy_oas [hy_oas]         2.79%   1年 p30 · 全庫 p18              自2023・日頻\n  ig_oas [ig_oas]         0.80%   1年 p60 · 全庫 p22              自2023・日頻\n\n殖利率(含無風險利率 —— 高低多半是利率在動,不是信用在動)\n  aaa [aaa]               5.52%   1年 p84 · 10年 p96 · 全庫 p61   自1919・月頻\n  baa [baa]               6.00%   1年 p46 · 10年 p86 · 全庫 p46   自1919・月頻\n  hy_yield [hy_yield]     7.19%   1年 p97 · 全庫 p93              自2023・日頻\n  ig_yield [ig_yield]     5.43%   1年 p99 · 全庫 p96              自2023・日頻\n  ccc_yield [ccc_yield]  14.28%   1年 p99 · 全庫 p93              自2023・日頻\n\n資料:日 至 2026-07-24(7 天前) · 月 至 2026-06\nSIGNAL-ONLY:百分位 = 在那個窗口裡排第幾,換一把尺就換一個答案。\n例:baa10y 1.59% —— 1年 排 p14,10年 排 p10。不是兩個市場,是兩把尺。";

// ── exact shape ──────────────────────────────────────────────────────────

#[test]
fn golden_message_matches_plan_exactly() {
    let rendered = render_lines(&golden_lines(), "2026-07-31");
    assert_eq!(
        rendered, GOLDEN,
        "rendered message must match the plan golden byte-for-byte"
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
        vec!["1年", "全庫"],
        "10y must be omitted for short coverage, not listed; got {labels:?}"
    );

    let msg = render_lines(&lines, "2026-07-31");
    assert!(
        !msg.contains("insufficient-coverage"),
        "must never print insufficient-coverage in the message: {msg}"
    );
    assert!(
        !msg.contains("10年"),
        "unreachable 10y window must not appear: {msg}"
    );
    assert!(msg.contains("1年 p"), "1年 must still appear: {msg}");
    assert!(msg.contains("全庫 p"), "全庫 must still appear: {msg}");
    // Omission is legible because coverage start is on the same line.
    assert!(
        msg.contains("自2023・日頻"),
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
        vec!["1年", "10年", "全庫"],
        "baa−aaa reaches 1919 so it shows all three windows; got {labels:?}"
    );
}

// ── precision ────────────────────────────────────────────────────────────

#[test]
fn precision_value_two_decimals_percentile_whole_number() {
    let line = SeriesLine {
        key: "ig_oas".into(),
            label: "ig_oas".into(),
        kind: SeriesKind::Spread,
        value: Some(0.8),
        windows: vec![
            WindowPct {
                label: "1年",
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
        msg.contains("0.80%"),
        "value must be two decimal places: {msg}"
    );
    // A percentile is a rank, so it shows as a whole number. `p60.0` implied a
    // precision the rank does not have; 22.14 becomes p22, not p22.1.
    assert!(msg.contains("p60"), "percentile is a whole number: {msg}");
    assert!(msg.contains("p22"), "percentile rounds to whole: {msg}");
    // Not unrounded / decimal forms.
    assert!(!msg.contains("0.800"), "{msg}");
    assert!(!msg.contains("p22.1"), "{msg}");
    assert!(!msg.contains("p60.0"), "{msg}");
}

// ── units ────────────────────────────────────────────────────────────────

#[test]
fn percent_marks_values_but_never_percentiles() {
    // This replaced a blanket "no % anywhere" rule. That rule existed because a
    // percentile then rendered as `p12.7`, and a decimal next to `2.84%` invited
    // reading the rank as a rate. Percentiles are whole numbers now, so the
    // confusion it guarded against is much weaker — while a bare `2.84` is
    // genuinely ambiguous, since OAS is commonly quoted in basis points (2.84%
    // vs 284bp is a factor of 100).
    //
    // The original worry still binds in its precise form: a percentile must
    // never carry a % sign.
    let msg = render_lines(&golden_lines(), "2026-07-31");
    assert!(msg.contains("0.48%"), "values carry the unit: {msg}");
    assert!(
        !regex_like_pct_with_percent(&msg),
        "a percentile must never be printed with a % sign: {msg}"
    );
}

/// True if any `pNN` token is immediately followed by `%`.
fn regex_like_pct_with_percent(msg: &str) -> bool {
    let b: Vec<char> = msg.chars().collect();
    for i in 0..b.len() {
        if b[i] != 'p' {
            continue;
        }
        let mut j = i + 1;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if j > i + 1 && j < b.len() && b[j] == '%' {
            return true;
        }
    }
    false
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
        msg.contains("自1986・日頻"),
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
        msg.contains("資料:日 至 2026-07-24(7 天前)"),
        "daily latest and age-in-days required: {msg}"
    );
    assert!(
        msg.contains("月 至 2026-06"),
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
        msg.contains("月 至 2026-06"),
        "monthly freshness must be the MIN latest (derived lags at 2026-06-01): {msg}"
    );
    assert!(
        !msg.contains("月 至 2026-07"),
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
        msg.lines().any(|l| l.contains("hy_oas") && l.contains("n/a") && l.contains("日頻")),
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
    let spread_hdr = msg.find("利差(已扣掉無風險利率 —— 這是信用風險本身的價格)").expect("spread header");
    let yield_hdr = msg
        .find("殖利率(含無風險利率 —— 高低多半是利率在動,不是信用在動)")
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
        msg.contains("SIGNAL-ONLY:百分位 = 在那個窗口裡排第幾,換一把尺就換一個答案。"),
        "{msg}"
    );
    assert!(
        !msg.contains("狀態：") && !msg.contains("狀態:"),
        "cds-con deliberately has no 狀態 line: {msg}"
    );
}

#[test]
fn percentile_never_displays_p100_by_rounding_up() {
    // 99.6 must not render as p100: that asserts nothing in the window sits
    // above this value, while 0.4% of it does. Truncation understates by under
    // one percentile and stays true.
    let line = SeriesLine {
        key: "ccc_yield".into(),
        label: "ccc_yield".into(),
        kind: SeriesKind::Yield,
        value: Some(14.28),
        windows: vec![WindowPct { label: "1年", pctile: 99.6 }],
        coverage_start: Some("2023-07-28".into()),
        latest: Some("2026-07-24".into()),
        frequency: Frequency::Daily,
        config_order: 0,
    };
    let msg = render_lines(&[line], "2026-07-31");
    assert!(msg.contains("p99"), "99.6 truncates to p99: {msg}");
    assert!(!msg.contains("p100"), "must never claim the top of the window: {msg}");
}

#[test]
fn cjk_labels_keep_columns_aligned() {
    // Char-based padding collapses here: 「品質利差」 is 4 chars but 8 columns.
    let mk = |label: &str, v: f64| SeriesLine {
        key: "k".into(),
        label: label.into(),
        kind: SeriesKind::Spread,
        value: Some(v),
        windows: vec![WindowPct { label: "1年", pctile: 50.0 }],
        coverage_start: Some("2023-07-28".into()),
        latest: Some("2026-07-24".into()),
        frequency: Frequency::Daily,
        config_order: 0,
    };
    let msg = render_lines(&[mk("品質利差 Baa−Aaa", 0.48), mk("ig_oas", 0.80)], "2026-07-31");
    let rows: Vec<&str> = msg.lines().filter(|l| l.contains("自2023")).collect();
    assert_eq!(rows.len(), 2);
    let col = |l: &str| l.find("自2023").unwrap();
    // Same display column, even though the byte offsets differ wildly.
    let width = |l: &str| -> usize {
        l[..col(l)].chars().map(|c| {
            let c = c as u32;
            let wide = (0x1100..=0x115F).contains(&c) || (0x2E80..=0xA4CF).contains(&c)
                || (0xAC00..=0xD7A3).contains(&c) || (0xF900..=0xFAFF).contains(&c)
                || (0xFE30..=0xFE6F).contains(&c) || (0xFF00..=0xFF60).contains(&c);
            if wide { 2 } else { 1 }
        }).sum()
    };
    assert_eq!(width(rows[0]), width(rows[1]), "coverage column must align:\n{msg}");
    assert_ne!(col(rows[0]), col(rows[1]), "byte offsets differ — proves the test is not trivial");
}
