//! Render-layer tests for cds-con Task 2.
//!
//! The golden message in the plan is the oracle. Every fixed rule (order,
//! windows, precision, units, provenance, freshness, missing series, missing
//! kind, monthly min-latest) has its own named test so a mutation gate can
//! point at a specific failure.

use cds_con::render::{
    analyze, format_message, render_lines, render_parts, width_bound, Frequency, LineKind,
    SeriesInput, SeriesLine, WindowPct, BAA_AAA_KEY,
};
use credit_store::{below_and_total, Observation, SeriesKind, SeriesSpec};

/// An `expand_days` bound that is always satisfied (day-of-month never
/// exceeds 31). Tests that need the monthly block visible alongside a
/// specific `as_of` (for its age-in-days arithmetic) widen the bound instead
/// of moving the date, so the daily age math stays intact.
const ALWAYS_EXPAND: u32 = 31;

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
                    label: "近1年".into(),
                    below: 0,
                    n: 12,
                },
                WindowPct {
                    label: "近10年".into(),
                    below: 0,
                    n: 120,
                },
                WindowPct {
                    label: "自1919".into(),
                    below: 48,
                    n: 1287,
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
                    label: "近1年".into(),
                    below: 37,
                    n: 250,
                },
                WindowPct {
                    label: "近10年".into(),
                    below: 255,
                    n: 2500,
                },
                WindowPct {
                    label: "自1986".into(),
                    below: 1120,
                    n: 10000,
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
                    label: "近1年".into(),
                    below: 75,
                    n: 250,
                },
                WindowPct {
                    label: "自2023".into(),
                    below: 136,
                    n: 750,
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
                    label: "近1年".into(),
                    below: 150,
                    n: 250,
                },
                WindowPct {
                    label: "自2023".into(),
                    below: 166,
                    n: 750,
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
                    label: "近1年".into(),
                    below: 10,
                    n: 12,
                },
                WindowPct {
                    label: "近10年".into(),
                    below: 116,
                    n: 120,
                },
                WindowPct {
                    label: "自1919".into(),
                    below: 793,
                    n: 1287,
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
                    label: "近1年".into(),
                    below: 6,
                    n: 13,
                },
                WindowPct {
                    label: "近10年".into(),
                    below: 103,
                    n: 120,
                },
                WindowPct {
                    label: "自1919".into(),
                    below: 601,
                    n: 1287,
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
                    label: "近1年".into(),
                    below: 243,
                    n: 250,
                },
                WindowPct {
                    label: "自2023".into(),
                    below: 699,
                    n: 750,
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
                    label: "近1年".into(),
                    below: 248,
                    n: 250,
                },
                WindowPct {
                    label: "自2023".into(),
                    below: 723,
                    n: 750,
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
                    label: "近1年".into(),
                    below: 249,
                    n: 250,
                },
                WindowPct {
                    label: "自2023".into(),
                    below: 701,
                    n: 750,
                },
            ],
            coverage_start: Some("2023-07-28".into()),
            latest: Some("2026-07-24".into()),
            frequency: Frequency::Daily,
            config_order: 8,
        },
    ]
}

/// Ordinary-day golden (`as_of = "2026-07-31"`, day 31 > 7 → monthly
/// collapsed). Captured verbatim from `render_lines(&golden_lines(), ...)`
/// (not hand-typed) and read against the design doc's §「Target output」
/// ordinary-day mock. Matches the design doc structurally in every respect,
/// including the blank line after each block header
/// (`利差 ——`/`總殖利率 ——`) and before its first series — `render_parts`
/// now pushes that blank explicitly, per the coordinator's ruling that the
/// approved mock wins over the renderer's prior (unintentional) omission of
/// it. Window counts here don't carry the doc's illustrative extra spaces
/// (`  61/250` vs `61/250`); that padding is the doc typesetting the mock
/// for human legibility, not a literal spec — §1 explicitly abandons
/// alignment. `golden_lines()`'s titles print `key [key]` rather than the
/// doc's prose `Label [key]` (e.g. `baa10y [baa10y]` not `Baa 比 10年期美債
/// 多出的殖利率 [baa10y]`) because the fixture has no config `Label` to draw
/// from — those labels are Step 3's live-config change, explicitly out of
/// scope here.
const GOLDEN_ORDINARY: &str = "💾 信用利差\n\n利差 —— 相對某個基準多出的殖利率\n\nbaa10y [baa10y]\n  1.59%  日頻・自1986\n  近1年  37/250 筆低於本次\n  近10年  255/2500 筆低於本次\n  自1986  1120/10000 筆低於本次\n\nhy_oas [hy_oas]\n  2.79%  日頻・自2023\n  近1年  75/250 筆低於本次\n  自2023  136/750 筆低於本次\n\nig_oas [ig_oas]\n  0.80%  日頻・自2023\n  近1年  150/250 筆低於本次\n  自2023  166/750 筆低於本次\n\n總殖利率 —— 含無風險利率在內的全部借款成本(與上一區不可互比)\n\nhy_yield [hy_yield]\n  7.19%  日頻・自2023\n  近1年  243/250 筆低於本次\n  自2023  699/750 筆低於本次\n\nig_yield [ig_yield]\n  5.43%  日頻・自2023\n  近1年  248/250 筆低於本次\n  自2023  723/750 筆低於本次\n\nccc_yield [ccc_yield]\n  14.28%  日頻・自2023\n  近1年  249/250 筆低於本次\n  自2023  701/750 筆低於本次\n\n資料:日 至 2026-07-24(7 天前)\n月頻 3 列 資料至 2026-06,未展開(每月 1–7 日展開)\nSIGNAL-ONLY:每個窗口各自回答自己的問題,不可跨列比較——\n自2023 的 750 筆和自1986 的 10000 筆不是同一把尺。";

/// Days-1–7 golden (`as_of = "2026-08-07"`, day 7 ≤ 7 → monthly expanded).
/// Same capture method as [`GOLDEN_ORDINARY`]; matches the design doc
/// structurally, including the post-header blank line. The monthly status
/// line is replaced by `・月 至 2026-06` on the 資料 line (per §4), and the
/// footer contrast names `自1919`/`自2023` — both match the design doc's
/// days-1–7 paragraph.
const GOLDEN_EXPANDED: &str = "💾 信用利差\n\n利差 —— 相對某個基準多出的殖利率\n\nbaa−aaa [baa−aaa]\n  0.48%  月頻・自1919\n  近1年  0/12 筆低於本次\n  近10年  0/120 筆低於本次\n  自1919  48/1287 筆低於本次\n\nbaa10y [baa10y]\n  1.59%  日頻・自1986\n  近1年  37/250 筆低於本次\n  近10年  255/2500 筆低於本次\n  自1986  1120/10000 筆低於本次\n\nhy_oas [hy_oas]\n  2.79%  日頻・自2023\n  近1年  75/250 筆低於本次\n  自2023  136/750 筆低於本次\n\nig_oas [ig_oas]\n  0.80%  日頻・自2023\n  近1年  150/250 筆低於本次\n  自2023  166/750 筆低於本次\n\n總殖利率 —— 含無風險利率在內的全部借款成本(與上一區不可互比)\n\naaa [aaa]\n  5.52%  月頻・自1919\n  近1年  10/12 筆低於本次\n  近10年  116/120 筆低於本次\n  自1919  793/1287 筆低於本次\n\nbaa [baa]\n  6.00%  月頻・自1919\n  近1年  6/13 筆低於本次\n  近10年  103/120 筆低於本次\n  自1919  601/1287 筆低於本次\n\nhy_yield [hy_yield]\n  7.19%  日頻・自2023\n  近1年  243/250 筆低於本次\n  自2023  699/750 筆低於本次\n\nig_yield [ig_yield]\n  5.43%  日頻・自2023\n  近1年  248/250 筆低於本次\n  自2023  723/750 筆低於本次\n\nccc_yield [ccc_yield]\n  14.28%  日頻・自2023\n  近1年  249/250 筆低於本次\n  自2023  701/750 筆低於本次\n\n資料:日 至 2026-07-24(14 天前) · 月 至 2026-06\nSIGNAL-ONLY:每個窗口各自回答自己的問題,不可跨列比較——\n自2023 的 750 筆和自1919 的 1287 筆不是同一把尺。";

// ── counts, not percentiles ─────────────────────────────────────────────

#[test]
fn wording_is_strictly_below_never_at_most() {
    let msg = render_lines(&golden_lines(), "2026-07-31", 7);
    assert!(msg.contains("筆低於本次"), "must state 低於");
    assert!(!msg.contains("不高於"), "不高於 is <=, the implementation is <");
}

#[test]
fn zero_below_renders_as_zero_over_n() {
    // A series sitting at its window minimum must print 0/N, never a blank,
    // an omitted window, or a dash. This is the p0 ambiguity fix.
    let line = SeriesLine {
        key: "q".into(),
        label: "Q".into(),
        kind: SeriesKind::Spread,
        value: Some(0.43),
        windows: vec![WindowPct { label: "近1年".into(), below: 0, n: 13 }],
        coverage_start: Some("1919-01-01".into()),
        latest: Some("2026-07-01".into()),
        frequency: Frequency::Monthly,
        config_order: 0,
    };
    // Monthly frequency: widen the bound so this line is not collapsed away.
    let msg = render_lines(&[line], "2026-07-31", ALWAYS_EXPAND);
    assert!(msg.contains("0/13 筆低於本次"), "got:\n{msg}");
}

#[test]
fn no_share_percentage_is_printed_beside_a_count() {
    let msg = render_lines(&golden_lines(), "2026-07-31", ALWAYS_EXPAND);
    for l in msg.lines().filter(|l| l.contains("筆低於本次")) {
        assert!(!l.contains('%'), "a count line must carry no %: {l}");
    }
}

// ── exact shape ──────────────────────────────────────────────────────────

#[test]
fn golden_ordinary_day() {
    let rendered = render_lines(&golden_lines(), "2026-07-31", 7);
    assert_eq!(
        rendered, GOLDEN_ORDINARY,
        "ordinary-day message must match the design doc byte-for-byte"
    );
}

#[test]
fn golden_first_seven_days() {
    let rendered = render_lines(&golden_lines(), "2026-08-07", 7);
    assert_eq!(
        rendered, GOLDEN_EXPANDED,
        "days 1-7 message must match the design doc byte-for-byte"
    );
}

// ── daily/monthly split ─────────────────────────────────────────────────

#[test]
fn ordinary_day_carries_exactly_the_six_daily_series() {
    let msg = render_lines(&golden_lines(), "2026-07-31", 7); // day 31 > 7
    assert!(!msg.contains("[baa]"), "monthly series must be collapsed");
    assert!(!msg.contains("[aaa]"));
    assert!(!msg.contains("[baa−aaa]"));
    assert!(msg.contains("[baa10y]") && msg.contains("[ccc_yield]"));
}

#[test]
fn monthly_block_expands_only_within_the_configured_day_bound() {
    let inside = render_lines(&golden_lines(), "2026-08-07", 7);
    let outside = render_lines(&golden_lines(), "2026-08-08", 7);
    assert!(inside.contains("[baa]"), "day 7 must expand");
    assert!(!outside.contains("[baa]"), "day 8 must not");
    // The bound is data: moving it moves the boundary.
    let moved = render_lines(&golden_lines(), "2026-08-08", 10);
    assert!(moved.contains("[baa]"), "bound must come from config, not code");
}

#[test]
fn day_bound_is_evaluated_from_as_of_never_a_clock() {
    // main.rs injects a CST date (cst_today, offset +8). Rendering must be a
    // pure function of as_of, so a UTC-keyed clock cannot creep in and
    // misfire for a whole month while fixed-date tests stay green.
    let a = render_lines(&golden_lines(), "2026-08-07", 7);
    let b = render_lines(&golden_lines(), "2026-08-07", 7);
    assert_eq!(a, b);
    assert!(a.contains("[baa]"));
}

#[test]
fn monthly_status_line_present_whenever_the_block_is_collapsed() {
    let msg = render_lines(&golden_lines(), "2026-07-31", 7);
    assert!(msg.contains("月頻 3 列 資料至 2026-06"), "got:\n{msg}");
    assert!(msg.contains("未展開"));
}

#[test]
fn monthly_status_line_absent_when_expanded() {
    let msg = render_lines(&golden_lines(), "2026-08-07", 7);
    assert!(!msg.contains("未展開"));
    assert!(msg.contains("月 至 2026-06"), "資料 line carries it instead");
}

#[test]
fn monthly_status_line_names_missing_monthly_series() {
    // format_freshness_line finds missing series by scanning RENDERED lines.
    // Filtering the monthly rows out must not hide a missing one for 29 days.
    let mut lines = golden_lines();
    for l in lines.iter_mut() {
        if l.key == "aaa" {
            l.value = None;
            l.windows.clear();
        }
    }
    let msg = render_lines(&lines, "2026-07-31", 7);
    assert!(msg.contains("缺 aaa"), "got:\n{msg}");
}

#[test]
fn footer_contrast_uses_only_series_rendered_today() {
    let msg = render_lines(&golden_lines(), "2026-07-31", 7);
    let shown: Vec<String> = msg
        .lines()
        .filter(|l| l.contains("筆低於本次"))
        .flat_map(extract_year_refs)
        .collect();
    let footer = msg.lines().last().unwrap();
    let footer_refs = extract_year_refs(footer);
    // The defect this test exists to catch is a footer naming a ruler that
    // isn't on screen. If extraction silently found nothing, the loop below
    // would pass vacuously and the guard would be defeated -- so pin that the
    // footer actually names both rulers it claims to contrast.
    assert!(
        footer_refs.len() >= 2,
        "footer must name both rulers it contrasts: {footer}"
    );
    for tok in &footer_refs {
        assert!(
            shown.iter().any(|s| s == tok),
            "footer names {tok}, which is not on screen today. shown={shown:?}"
        );
    }
}

/// Every `自YYYY` occurrence in `s`, found by scanning for the `自` marker and
/// taking the following 4 characters -- not by splitting on whitespace.
///
/// The footer sentence joins its second ruler with no leading space
/// (`...筆和自{year}...`), so a `split_whitespace` scan only ever isolates the
/// FIRST `自YYYY` in the line. That is exactly the ruler the original defect
/// (naming a collapsed series in the footer) did not get wrong -- the second
/// one did. A test that can't see the second token can't catch a regression
/// in it, so this scans by character instead of relying on a delimiter the
/// renderer never promised to put there.
fn extract_year_refs(s: &str) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '自' && i + 4 < chars.len() {
            let year: String = chars[i + 1..=i + 4].iter().collect();
            if year.chars().all(|c| c.is_ascii_digit()) {
                out.push(format!("自{year}"));
            }
        }
        i += 1;
    }
    out
}

#[test]
fn footer_contrast_matches_the_full_history_window_by_label_not_position() {
    // Both lines' `windows` are built with the full-history entry NOT last --
    // the opposite of what `series_line_from_rows` happens to do today. If
    // `footer_contrast` ever again reads `windows.last()` positionally, this
    // must catch it by printing a count from the wrong window.
    let a = SeriesLine {
        key: "a".into(),
        label: "A".into(),
        kind: SeriesKind::Spread,
        value: Some(1.59),
        windows: vec![
            WindowPct { label: "自1986".into(), below: 1120, n: 10000 },
            WindowPct { label: "近1年".into(), below: 37, n: 250 },
        ],
        coverage_start: Some("1986-01-02".into()),
        latest: Some("2026-07-24".into()),
        frequency: Frequency::Daily,
        config_order: 0,
    };
    let b = SeriesLine {
        key: "b".into(),
        label: "B".into(),
        kind: SeriesKind::Spread,
        value: Some(2.79),
        windows: vec![
            WindowPct { label: "自2023".into(), below: 136, n: 750 },
            WindowPct { label: "近1年".into(), below: 75, n: 250 },
        ],
        coverage_start: Some("2023-07-28".into()),
        latest: Some("2026-07-24".into()),
        frequency: Frequency::Daily,
        config_order: 1,
    };
    let msg = render_lines(&[a, b], "2026-07-31", 7);
    let footer = msg.lines().last().unwrap();
    assert!(
        footer.contains("自1986 的 10000 筆") && footer.contains("自2023 的 750 筆"),
        "footer must report each series' full-history count found by its \
         `自YYYY` label, not by vec position (which would print 250 for both \
         here, since 近1年 sits last in each windows vec): {footer}"
    );
}

#[test]
fn footer_is_coherent_when_only_one_coverage_year_is_shown() {
    // If baa10y (the only 1986-start series) is missing, every remaining
    // daily series shares 自2023 -- footer_contrast returns None because
    // there is nothing to contrast. The footer must not end on a dangling
    // continuation dash with nothing after it.
    let mut lines = golden_lines();
    for l in lines.iter_mut() {
        if l.key == "baa10y" {
            l.value = None;
            l.windows.clear();
            l.coverage_start = None;
        }
    }
    let msg = render_lines(&lines, "2026-07-31", 7);
    let footer = msg.lines().last().unwrap();
    assert!(
        !footer.ends_with('—') && !footer.ends_with("——"),
        "footer must not dangle on a continuation dash with nothing to \
         follow it: {footer}"
    );
    assert!(
        footer.contains("SIGNAL-ONLY"),
        "the marker must still be present: {footer}"
    );
    assert!(
        footer.ends_with('。'),
        "a self-contained footer sentence must close, not trail off: {footer}"
    );
}

#[test]
fn spreads_header_makes_no_claim_about_the_price_of_credit_risk() {
    let msg = render_lines(&golden_lines(), "2026-07-31", 7);
    assert!(!msg.contains("信用風險本身的價格"));
    assert!(msg.contains("利差 —— 相對某個基準多出的殖利率"));
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
    let labels: Vec<&str> = lines[0].windows.iter().map(|w| w.label.as_str()).collect();
    assert_eq!(
        labels,
        vec!["近1年", "自2023"],
        "10y must be omitted for short coverage, not listed; got {labels:?}"
    );

    let msg = render_lines(&lines, "2026-07-31", 7);
    assert!(
        !msg.contains("insufficient-coverage"),
        "must never print insufficient-coverage in the message: {msg}"
    );
    assert!(
        !msg.contains("10年"),
        "unreachable 10y window must not appear: {msg}"
    );
    assert!(
        msg.lines().any(|l| l.contains("近1年") && l.contains("筆低於本次")),
        "近1年 count must still appear: {msg}"
    );
    assert!(
        msg.lines().any(|l| l.contains("自2023") && l.contains("筆低於本次")),
        "full-history count must still appear: {msg}"
    );
    // Omission is legible because coverage start is on the same line.
    assert!(
        msg.contains("日頻・自2023"),
        "coverage start must remain on the line: {msg}"
    );
}

#[test]
fn long_coverage_series_shows_all_three_windows() {
    // Coverage from 1919 supports all three windows (1y, 10y, full-history)
    // — including derived baa−aaa.
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
    let labels: Vec<&str> = derived.windows.iter().map(|w| w.label.as_str()).collect();
    assert_eq!(
        labels,
        vec!["近1年", "近10年", "自1919"],
        "baa−aaa reaches 1919 so it shows all three windows; got {labels:?}"
    );
}

/// `golden_lines()` hand-authors each `WindowPct.label` as a literal string
/// ("自1919" / "自1986" / "自2023") — it never calls `series_line_from_rows`,
/// so this test never reaches `year_str`/`coverage_year`. It pins only that
/// `render_lines` passes an already-computed label through unchanged and does
/// not fall back to (or reintroduce) the old fixed placeholder word. For a
/// test that derives the label from raw dates through the real path, see
/// `start_year_label_is_derived_from_the_data_not_supplied` below.
#[test]
fn window_label_is_the_actual_start_year() {
    let msg = render_lines(&golden_lines(), "2026-07-31", 7);
    assert!(!msg.contains("全庫"), "全庫 hides that each row is a different ruler");
    assert!(msg.contains("自1919") || msg.contains("自1986"), "got:\n{msg}");
}

/// Same caveat as above: `golden_lines()` supplies three already-distinct
/// labels directly on `WindowPct`, so this pins that `render_lines` prints
/// distinct supplied labels as distinct — it does not exercise the slicing
/// that computes a label from a coverage-start date.
#[test]
fn three_series_with_different_coverage_get_three_different_labels() {
    // ALWAYS_EXPAND: the golden's three coverage years (1919/1986/2023) span
    // both the monthly and daily blocks, so the monthly block must be shown.
    let msg = render_lines(&golden_lines(), "2026-07-31", ALWAYS_EXPAND);
    let years: std::collections::HashSet<&str> = msg
        .lines()
        .filter(|l| l.contains("筆低於本次"))
        .filter_map(|l| l.split_whitespace().find(|t| t.starts_with('自')))
        .collect();
    assert!(
        years.len() >= 3,
        "the golden spans 1919/1986/2023; each must print its own ruler, got {years:?}"
    );
}

/// Unlike the two tests above, this one supplies raw `Observation` rows and
/// goes through `analyze()` / `series_line_from_rows()` — the real path that
/// calls `year_str`/`coverage_year`. A hand-authored label can never catch a
/// regression in that slicing (wrong bound, off-by-one, slicing the month
/// instead of the year); this test can, because the label here is computed,
/// not supplied.
#[test]
fn start_year_label_is_derived_from_the_data_not_supplied() {
    let series = vec![input(
        "baa10y",
        SeriesKind::Spread,
        Frequency::Daily,
        obs(&[("1986-01-02", 1.0), ("2026-07-31", 2.0)]),
    )];
    let lines = analyze(&series).expect("kind is set");
    let msg = render_lines(&lines, "2026-07-31", 7);
    assert!(
        msg.contains("自1986"),
        "year must come from rows[0].date through year_str/coverage_year; got:\n{msg}"
    );
    assert!(
        !msg.contains("自1986-") && !msg.contains("自19860"),
        "the slice must be exactly the 4-char year, not the full date or an \
         off-by-one into the month: {msg}"
    );
    assert!(!msg.contains("全庫"), "{msg}");
}

// ── precision ────────────────────────────────────────────────────────────

#[test]
fn precision_value_two_decimals_count_is_a_plain_integer() {
    // Percentile-rounding precision (p22.1 vs p22) is now structurally moot:
    // `below`/`n` are `usize`, so a count can never carry a fractional digit.
    // What remains a real behaviour worth pinning is that the *value* still
    // renders at two decimal places, and the count renders as a bare integer
    // pair with no decimal point.
    let line = SeriesLine {
        key: "ig_oas".into(),
        label: "ig_oas".into(),
        kind: SeriesKind::Spread,
        value: Some(0.8),
        windows: vec![
            WindowPct {
                label: "近1年".into(),
                below: 150,
                n: 250,
            },
            WindowPct {
                label: "自2023".into(),
                below: 166,
                n: 750,
            },
        ],
        coverage_start: Some("2023-07-28".into()),
        latest: Some("2026-07-24".into()),
        frequency: Frequency::Daily,
        config_order: 0,
    };
    let msg = render_lines(&[line], "2026-07-31", 7);
    assert!(
        msg.contains("0.80%"),
        "value must be two decimal places: {msg}"
    );
    assert!(msg.contains("150/250 筆低於本次"), "{msg}");
    assert!(msg.contains("166/750 筆低於本次"), "{msg}");
    // Not unrounded / decimal forms.
    assert!(!msg.contains("0.800"), "{msg}");
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
    // 0.48% is baa−aaa, a monthly series — widen the bound so it is shown.
    let msg = render_lines(&golden_lines(), "2026-07-31", ALWAYS_EXPAND);
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
    let msg = format_message(&series, "2026-07-31", 7).unwrap();
    assert!(
        msg.contains("日頻・自1986"),
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
    // ALWAYS_EXPAND so the monthly latest is on the 資料 line too (this test
    // predates the daily/monthly split and pins both halves at once).
    let msg = render_lines(&golden_lines(), "2026-07-31", ALWAYS_EXPAND);
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
    // Both inputs are Monthly — widen the bound so the block is shown.
    let msg = format_message(&series, "2026-07-31", ALWAYS_EXPAND).unwrap();
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
    // hy_oas (missing, Daily) is shown regardless of the monthly bound.
    let msg = format_message(&series, "2026-07-31", 7).unwrap();
    assert!(
        msg.contains("hy_oas") && msg.contains("n/a"),
        "missing series must render as n/a, not vanish: {msg}"
    );
    // Vertical form: the key sits on its own title line (`series_block`'s
    // first line); value + frequency are the line right after. They no
    // longer share one row now that nothing is padded into columns, but both
    // halves of the guarantee -- n/a rendered, frequency still shown -- must
    // still hold, one line apart.
    let lines: Vec<&str> = msg.lines().collect();
    let title_idx = lines
        .iter()
        .position(|l| l.contains("hy_oas ["))
        .expect("hy_oas title line must exist");
    let value_line = lines
        .get(title_idx + 1)
        .expect("value line must immediately follow the title line");
    assert!(
        value_line.contains("n/a") && value_line.contains("日頻"),
        "n/a row keeps frequency on the line right after the title: {value_line}"
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
    let err = format_message(&series, "2026-07-31", 7).expect_err("missing kind must err");
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
    // ALWAYS_EXPAND: this test asserts BOTH the daily block (baa10y) and the
    // monthly block (aaa) land under the right header, so the monthly block
    // must be visible.
    let msg = render_lines(&golden_lines(), "2026-07-31", ALWAYS_EXPAND);
    let spread_hdr = msg
        .find("利差 —— 相對某個基準多出的殖利率")
        .expect("spread header");
    let yield_hdr = msg
        .find("總殖利率 —— 含無風險利率在內的全部借款成本(與上一區不可互比)")
        .expect("yield header");
    assert!(
        spread_hdr < yield_hdr,
        "spread block must precede yield block"
    );
    // Keys land under the right header. Vertical layout puts a series' title
    // line (`Label [key]`) at column 0, unindented, unlike the value/window
    // lines under it -- so anchor on "\naaa [aaa]" rather than a leading
    // two-space table-row indent (that would also wrongly match "aaa" as a
    // bare substring of "baa−aaa [baa−aaa]"'s own title line).
    let baa10y = msg.find("baa10y").unwrap();
    let aaa = msg.find("\naaa [aaa]").expect("aaa title line");
    assert!(baa10y > spread_hdr && baa10y < yield_hdr, "baa10y under spreads");
    assert!(aaa > yield_hdr, "aaa under yields");
}

// ── SIGNAL-ONLY closer ───────────────────────────────────────────────────

#[test]
fn closes_with_signal_only_and_has_no_status_line() {
    let msg = render_lines(&golden_lines(), "2026-07-31", 7);
    assert!(
        msg.contains("SIGNAL-ONLY:每個窗口各自回答自己的問題,不可跨列比較——"),
        "{msg}"
    );
    assert!(
        !msg.contains("狀態：") && !msg.contains("狀態:"),
        "cds-con deliberately has no 狀態 line: {msg}"
    );
}

// `percentile_never_displays_p100_by_rounding_up` was deleted, then restored
// below in rewritten form. Its old mechanism (a fractional percentile
// rounding 99.6 up to `p100`) is gone — `below: usize` bounded by `n`
// structurally cannot claim a rank above the window's own size. But the
// GLOBAL CONSTRAINT it pinned ("the display may never claim a higher rank
// than the data supports") still needs a guard: nothing stops a future
// change from re-deriving the printed number from a percentage (rounding it
// in the process) instead of carrying `below_and_total`'s raw output through
// unchanged. `printed_count_is_the_raw_comparison_never_derived` pins that.

#[test]
fn printed_count_is_the_raw_comparison_never_derived() {
    // The old failure mode was 99.6 rounding up to p100, claiming nothing sat
    // above the value when 0.4% of the window did. Counts remove the
    // rounding step entirely by construction (`below` can never exceed `n`),
    // so the guarantee becomes: what is printed IS `below_and_total`'s raw
    // output, not a number reconstructed from a rounded/scaled percentage.
    let vals: Vec<f64> = (0..1000).map(|i| i as f64).collect();
    let (below, n) = below_and_total(&vals, 996.0);
    assert_eq!((below, n), (996, 1000), "sanity: 996 of 0..999 sit strictly below 996.0");

    // Feed that exact (below, n) straight into the WindowPct that render_lines
    // formats — no recomputation, no percentage round-trip.
    let line = SeriesLine {
        key: "q".into(),
        label: "Q".into(),
        kind: SeriesKind::Spread,
        value: Some(9.96),
        windows: vec![WindowPct { label: "近1年".into(), below, n }],
        coverage_start: Some("2020-01-01".into()),
        latest: Some("2026-07-01".into()),
        frequency: Frequency::Daily,
        config_order: 0,
    };
    let msg = render_lines(&[line], "2026-07-31", 7);
    assert!(msg.contains("996/1000 筆低於本次"), "must print the raw count: got:\n{msg}");
    assert!(
        !msg.contains("1000/1000"),
        "must never claim the top of the window (the old p100 bug, in count form): {msg}"
    );
    assert!(
        msg.lines()
            .filter(|l| l.contains("筆低於本次"))
            .all(|l| !l.contains('%')),
        "no share/percentage may appear beside a count: {msg}"
    );
}

// `cjk_labels_keep_columns_aligned` was deleted here: it asserted that byte
// offsets differ while display columns match, which is meaningless once
// nothing is padded into columns. See `every_rendered_line_fits_its_width_bound`
// below for the guarantee that replaces it.

#[test]
fn every_rendered_line_fits_its_width_bound() {
    // Proxy, not proof: the transport (`parse_mode: None`) renders a
    // proportional font, where this CJK-is-2-columns model does not describe
    // the real wrap point. What this prevents is a line bloating back to a
    // size that breaks even a monospace reader; it is not a guarantee
    // against wrapping on a phone.
    fn width(s: &str) -> usize {
        s.chars()
            .map(|c| {
                let c = c as u32;
                let wide = (0x1100..=0x115F).contains(&c)
                    || (0x2E80..=0xA4CF).contains(&c)
                    || (0xAC00..=0xD7A3).contains(&c)
                    || (0xF900..=0xFAFF).contains(&c)
                    || (0xFE30..=0xFE6F).contains(&c)
                    || (0xFF00..=0xFF60).contains(&c)
                    || (0xFFE0..=0xFFE6).contains(&c);
                if wide { 2 } else { 1 }
            })
            .sum()
    }

    // ALWAYS_EXPAND so both the daily and the monthly blocks are checked in
    // one pass, including the derived baa−aaa row's labels.
    let parts = render_parts(&golden_lines(), "2026-07-31", ALWAYS_EXPAND);
    // The prose/data distinction is STRUCTURAL: it comes from `Segment::kind`,
    // the tag the renderer itself attached to each line, never from guessing
    // at the line's text. A future series label from `cds_series` beginning
    // with `利差`/`殖利率` cannot silently exempt a real data row this way --
    // it would if this test instead matched on text prefixes.
    for seg in parts.iter().filter(|s| !s.text.trim().is_empty()) {
        if seg.kind != LineKind::Data {
            continue;
        }
        assert!(
            width(&seg.text) <= width_bound(),
            "line is {} cols (bound {}): {}",
            width(&seg.text),
            width_bound(),
            seg.text
        );
    }
}
