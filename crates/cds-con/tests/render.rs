//! Render-layer tests for cds-con v3.
//!
//! The golden message (owner-approved, real data from 2026-07-30) is the
//! oracle for exact shape. Every fixed rule (order, windows, precision,
//! units, provenance, freshness, missing series, missing kind, message-series
//! selection) has its own named test so a mutation gate can point at a
//! specific failure.

use cds_con::render::{
    analyze, format_message, parse_message_series, render_lines, render_parts,
    select_message_series, width_bound, Frequency, LineKind, SeriesInput, SeriesLine, WindowPct,
    BAA_AAA_KEY, BAA_AAA_LABEL,
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

fn keys(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

/// General-purpose 9-series fixture (labels are just the key -- these tests
/// exercise layout/ordering/freshness rules, not the live Chinese labels,
/// which are config, not Rust). Spans three coverage years (1919/1986/2023)
/// across both frequencies so order/window/freshness rules all have
/// something to bite on. Unlike v2, nothing filters this list before
/// rendering -- v3 has no daily/monthly split, so every series here always
/// renders.
fn golden_lines() -> Vec<SeriesLine> {
    vec![
        SeriesLine {
            key: BAA_AAA_KEY.into(),
            label: BAA_AAA_KEY.into(),
            kind: SeriesKind::Spread,
            value: Some(0.48),
            windows: vec![
                WindowPct { label: "近1年".into(), below: 0, n: 12 },
                WindowPct { label: "近10年".into(), below: 0, n: 120 },
                WindowPct { label: "自1919".into(), below: 48, n: 1287 },
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
                WindowPct { label: "近1年".into(), below: 37, n: 250 },
                WindowPct { label: "近10年".into(), below: 255, n: 2500 },
                WindowPct { label: "自1986".into(), below: 1120, n: 10000 },
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
                WindowPct { label: "近1年".into(), below: 75, n: 250 },
                WindowPct { label: "自2023".into(), below: 136, n: 750 },
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
                WindowPct { label: "近1年".into(), below: 150, n: 250 },
                WindowPct { label: "自2023".into(), below: 166, n: 750 },
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
                WindowPct { label: "近1年".into(), below: 10, n: 12 },
                WindowPct { label: "近10年".into(), below: 116, n: 120 },
                WindowPct { label: "自1919".into(), below: 793, n: 1287 },
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
                WindowPct { label: "近1年".into(), below: 6, n: 13 },
                WindowPct { label: "近10年".into(), below: 103, n: 120 },
                WindowPct { label: "自1919".into(), below: 601, n: 1287 },
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
                WindowPct { label: "近1年".into(), below: 243, n: 250 },
                WindowPct { label: "自2023".into(), below: 699, n: 750 },
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
                WindowPct { label: "近1年".into(), below: 248, n: 250 },
                WindowPct { label: "自2023".into(), below: 723, n: 750 },
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
                WindowPct { label: "近1年".into(), below: 249, n: 250 },
                WindowPct { label: "自2023".into(), below: 701, n: 750 },
            ],
            coverage_start: Some("2023-07-28".into()),
            latest: Some("2026-07-24".into()),
            frequency: Frequency::Daily,
            config_order: 8,
        },
    ]
}

/// The exact five series `cds_message_series` shows (design doc v3 §1), with
/// the real 2026-07-30 values/counts the owner approved. Hand-authored
/// SeriesLines, not run through `analyze()` -- this is the layout oracle for
/// [`golden_message`], the byte-for-byte test.
fn v3_golden_lines() -> Vec<SeriesLine> {
    vec![
        SeriesLine {
            key: BAA_AAA_KEY.into(),
            label: BAA_AAA_LABEL.into(),
            kind: SeriesKind::Spread,
            value: Some(0.43),
            windows: vec![
                WindowPct { label: "近1年".into(), below: 0, n: 13 },
                WindowPct { label: "近10年".into(), below: 0, n: 121 },
                WindowPct { label: "自1919".into(), below: 22, n: 1291 },
            ],
            coverage_start: Some("1919-01-01".into()),
            latest: Some("2026-07-01".into()),
            frequency: Frequency::Monthly,
            config_order: 0,
        },
        SeriesLine {
            key: "baa10y".into(),
            label: "Baa 比 10年期美債多出的殖利率".into(),
            kind: SeriesKind::Spread,
            value: Some(1.63),
            windows: vec![
                WindowPct { label: "近1年".into(), below: 61, n: 250 },
                WindowPct { label: "近10年".into(), below: 316, n: 2495 },
                WindowPct { label: "自1986".into(), below: 1397, n: 10145 },
            ],
            coverage_start: Some("1986-01-02".into()),
            latest: Some("2026-07-30".into()),
            frequency: Frequency::Daily,
            config_order: 1,
        },
        SeriesLine {
            key: "hy_oas".into(),
            label: "高收益債相對基準多出的殖利率".into(),
            kind: SeriesKind::Spread,
            value: Some(2.84),
            windows: vec![
                WindowPct { label: "近1年".into(), below: 117, n: 265 },
                WindowPct { label: "自2023".into(), below: 194, n: 789 },
            ],
            coverage_start: Some("2023-07-28".into()),
            latest: Some("2026-07-30".into()),
            frequency: Frequency::Daily,
            config_order: 2,
        },
        SeriesLine {
            key: "ig_oas".into(),
            label: "投資級債相對基準多出的殖利率".into(),
            kind: SeriesKind::Spread,
            value: Some(0.80),
            windows: vec![
                WindowPct { label: "近1年".into(), below: 155, n: 265 },
                WindowPct { label: "自2023".into(), below: 173, n: 788 },
            ],
            coverage_start: Some("2023-07-28".into()),
            latest: Some("2026-07-30".into()),
            frequency: Frequency::Daily,
            config_order: 3,
        },
        SeriesLine {
            key: "baa".into(),
            label: "Baa 級公司債總殖利率".into(),
            kind: SeriesKind::Yield,
            value: Some(6.19),
            windows: vec![
                WindowPct { label: "近1年".into(), below: 12, n: 13 },
                WindowPct { label: "近10年".into(), below: 116, n: 121 },
                WindowPct { label: "自1919".into(), below: 652, n: 1291 },
            ],
            coverage_start: Some("1919-01-01".into()),
            latest: Some("2026-07-01".into()),
            frequency: Frequency::Monthly,
            config_order: 4,
        },
    ]
}

const GOLDEN: &str = "💾 信用利差 · 2026-07-30\n\n利差 —— 相對某個基準多出的殖利率\n已扣掉利率,動的是市場對「借錢給公司」的要價\n\nBaa 比 Aaa 多出的殖利率  0.43%   [baa−aaa]\n  近1年 13 筆裡 0 筆比現在低(0.0%)\n  近10年 121 筆裡 0 筆比現在低(0.0%)\n  自1919 1291 筆裡 22 筆比現在低(1.7%)\n\nBaa 比 10年期美債多出的殖利率  1.63%   [baa10y]\n  近1年 250 筆裡 61 筆比現在低(24.4%)\n  近10年 2495 筆裡 316 筆比現在低(12.6%)\n  自1986 10145 筆裡 1397 筆比現在低(13.7%)\n\n高收益債相對基準多出的殖利率  2.84%   [hy_oas]\n  近1年 265 筆裡 117 筆比現在低(44.1%)\n  自2023 789 筆裡 194 筆比現在低(24.5%)\n\n投資級債相對基準多出的殖利率  0.80%   [ig_oas]\n  近1年 265 筆裡 155 筆比現在低(58.4%)\n  自2023 788 筆裡 173 筆比現在低(21.9%)\n\n總殖利率 —— 含利率在內的全部借款成本,與上一區不可互比\n留這一條當對照:同一批 Baa 債,上面那條扣掉了利率,這條沒扣\n\nBaa 級公司債總殖利率  6.19%   [baa]\n  近1年 13 筆裡 12 筆比現在低(92.3%)\n  近10年 121 筆裡 116 筆比現在低(95.8%)\n  自1919 1291 筆裡 652 筆比現在低(50.5%)\n\n資料:日 至 2026-07-30(5 天前)・月 至 2026-07\nSIGNAL-ONLY:窗口越短對當下越敏感,越長越穩定。它們回答不同的問題,不可跨列比。";

// ── exact shape ──────────────────────────────────────────────────────────

#[test]
fn golden_message() {
    // as_of 2026-08-04 is 5 days after the daily latest (2026-07-30), matching
    // the freshness line's "(5 天前)" -- there is now only one message shape,
    // so only one golden (v2 had two, for the ordinary/expanded days a split
    // that no longer exists produced).
    let rendered = render_lines(&v3_golden_lines(), "2026-08-04");
    assert_eq!(
        rendered, GOLDEN,
        "the v3 message must match the owner-approved target byte-for-byte"
    );
}

// ── counts, share, truncation ───────────────────────────────────────────

#[test]
fn wording_is_strictly_below_never_at_most() {
    let msg = render_lines(&golden_lines(), "2026-07-31");
    assert!(msg.contains("筆比現在低"), "must state 低於 in some form");
    assert!(!msg.contains("不高於"), "不高於 is <=, the implementation is <");
}

#[test]
fn zero_below_renders_as_zero_count_not_omitted() {
    // A series sitting at its window minimum must print `N 筆裡 0 筆比現在低`,
    // never a blank, an omitted window, or a dash. This is the p0-ambiguity
    // fix carried over from v2: a count can never be ambiguous the way a
    // rounded percentile could.
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
    let msg = render_lines(&[line], "2026-07-31");
    assert!(msg.contains("13 筆裡 0 筆比現在低(0.0%)"), "got:\n{msg}");
}

#[test]
fn share_percent_is_truncated_never_rounded_up() {
    // 2/3 = 66.666...% -- standard 1-decimal rounding gives 66.7%, but v3 §3
    // requires truncation, so the correct printed share is 66.6%.
    let line = SeriesLine {
        key: "q".into(),
        label: "Q".into(),
        kind: SeriesKind::Spread,
        value: Some(1.00),
        windows: vec![WindowPct { label: "近1年".into(), below: 2, n: 3 }],
        coverage_start: Some("2020-01-01".into()),
        latest: Some("2026-07-01".into()),
        frequency: Frequency::Daily,
        config_order: 0,
    };
    let msg = render_lines(&[line], "2026-07-31");
    assert!(
        msg.contains("3 筆裡 2 筆比現在低(66.6%)"),
        "must truncate, not round: got:\n{msg}"
    );
    assert!(!msg.contains("66.7%"), "must never round up: {msg}");
}

#[test]
fn printed_count_is_the_raw_comparison_never_derived() {
    // The old (v2) failure mode was a fractional percentile rounding 99.6 up
    // to p100, claiming nothing sat above the value when 0.4% of the window
    // did. Counts remove the rounding step by construction (`below` can never
    // exceed `n`); what remains to guard is that the printed share is
    // computed from that same raw pair, never a separately reconstructed
    // percentage.
    let line = SeriesLine {
        key: "q".into(),
        label: "Q".into(),
        kind: SeriesKind::Spread,
        value: Some(9.96),
        windows: vec![WindowPct { label: "近1年".into(), below: 996, n: 1000 }],
        coverage_start: Some("2020-01-01".into()),
        latest: Some("2026-07-01".into()),
        frequency: Frequency::Daily,
        config_order: 0,
    };
    let msg = render_lines(&[line], "2026-07-31");
    assert!(
        msg.contains("1000 筆裡 996 筆比現在低(99.6%)"),
        "must print the raw count and its truncated share: got:\n{msg}"
    );
    assert!(
        !msg.contains("1000 筆裡 1000 筆比現在低"),
        "must never claim the top of the window (the old p100 bug, in count form): {msg}"
    );
    assert!(
        msg.lines()
            .filter(|l| l.contains("筆比現在低"))
            .all(|l| l.matches('%').count() == 1),
        "a window line carries exactly one %, the share: {msg}"
    );
}

// ── rate and share never share a line ───────────────────────────────────

/// Rewritten from `percent_marks_values_but_never_percentiles` (v2's rule --
/// "a percentile never carries `%`" -- is moot now that v3 puts a share back
/// beside every count). The *reason* behind that rule survives in a new
/// form: v3 §3 moved the value onto the title line specifically so a rate
/// (the title's `%`) and a share (a window line's `%`) never sit on the same
/// line, which is what made the v2-era collision (`(24.4%)` sitting one line
/// under `1.63%`) confusing in the first place.
#[test]
fn rate_and_share_never_share_a_line() {
    let msg = render_lines(&golden_lines(), "2026-07-31");
    let mut saw_rate_line = false;
    let mut saw_share_line = false;
    for line in msg.lines() {
        let pct_count = line.matches('%').count();
        assert!(
            pct_count <= 1,
            "a rate and a share must never appear on the same line: {line}"
        );
        if pct_count == 1 && line.contains('[') {
            saw_rate_line = true;
        }
        if pct_count == 1 && line.contains("筆比現在低") {
            saw_share_line = true;
        }
    }
    assert!(saw_rate_line, "expected at least one title (rate) line: {msg}");
    assert!(saw_share_line, "expected at least one window (share) line: {msg}");
}

// ── block headers ────────────────────────────────────────────────────────

#[test]
fn block_headers_carry_fixed_explanatory_prose() {
    // Fixed prose about what each block measures, not a read of today's
    // market shape -- these strings never interpolate a live number, so they
    // cannot become false on a day the market moves.
    let msg = render_lines(&golden_lines(), "2026-07-31");
    assert!(msg.contains("已扣掉利率,動的是市場對「借錢給公司」的要價"));
    assert!(msg.contains("留這一條當對照:同一批 Baa 債,上面那條扣掉了利率,這條沒扣"));
}

#[test]
fn spreads_header_makes_no_claim_about_the_price_of_credit_risk() {
    let msg = render_lines(&golden_lines(), "2026-07-31");
    assert!(!msg.contains("信用風險本身的價格"));
    assert!(msg.contains("利差 —— 相對某個基準多出的殖利率"));
}

// ── header date ──────────────────────────────────────────────────────────

#[test]
fn header_carries_the_most_recent_daily_date() {
    // golden_lines()'s daily series all latest at 2026-07-24; the header is a
    // fact about the data ("what date do these numbers reflect"), not the
    // run date.
    let msg = render_lines(&golden_lines(), "2026-07-31");
    assert!(
        msg.starts_with("💾 信用利差 · 2026-07-24"),
        "header must carry the daily latest date: {msg}"
    );
}

#[test]
fn header_falls_back_to_monthly_when_no_daily_series_is_shown() {
    let line = SeriesLine {
        key: "aaa".into(),
        label: "aaa".into(),
        kind: SeriesKind::Yield,
        value: Some(5.5),
        windows: vec![WindowPct { label: "自1919".into(), below: 1, n: 2 }],
        coverage_start: Some("1919-01-01".into()),
        latest: Some("2026-06-01".into()),
        frequency: Frequency::Monthly,
        config_order: 0,
    };
    let msg = render_lines(&[line], "2026-07-31");
    assert!(
        msg.starts_with("💾 信用利差 · 2026-06-01"),
        "must fall back to the monthly latest when no daily series is present: {msg}"
    );
}

// ── message-series config: parsing ──────────────────────────────────────

#[test]
fn message_series_parses_a_comma_list_in_display_order() {
    let got = parse_message_series("baa−aaa,baa10y,hy_oas,ig_oas,baa").unwrap();
    assert_eq!(got, vec!["baa−aaa", "baa10y", "hy_oas", "ig_oas", "baa"]);
}

#[test]
fn message_series_trims_whitespace_around_each_key() {
    let got = parse_message_series(" baa10y , hy_oas ,ig_oas").unwrap();
    assert_eq!(got, vec!["baa10y", "hy_oas", "ig_oas"]);
}

#[test]
fn message_series_empty_value_fails() {
    let err = parse_message_series("").unwrap_err();
    assert!(err.contains("empty"), "{err}");
    let err = parse_message_series("   ").unwrap_err();
    assert!(err.contains("empty"), "{err}");
}

#[test]
fn message_series_empty_token_fails() {
    // A doubled comma or a trailing comma leaves an empty token -- must fail
    // loudly rather than silently skip it (v3 §5 / task item 2).
    let err = parse_message_series("baa10y,,hy_oas").unwrap_err();
    assert!(err.contains("empty") && err.contains("position 2"), "{err}");
    let err = parse_message_series("baa10y,hy_oas,").unwrap_err();
    assert!(err.contains("empty"), "{err}");
}

// ── message-series config: selection ────────────────────────────────────

#[test]
fn select_message_series_orders_by_config_not_coverage() {
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
            obs(&[("2023-07-28", 2.0), ("2026-07-24", 2.79)]),
        ),
        input(
            "ig_oas",
            SeriesKind::Spread,
            Frequency::Daily,
            obs(&[("2023-07-28", 0.5), ("2026-07-24", 0.80)]),
        ),
    ];
    let lines = analyze(&series).unwrap();
    // analyze() itself would sort baa10y (1986) before hy_oas/ig_oas (2023).
    // The message config deliberately reverses that.
    let selected = select_message_series(&lines, &keys(&["ig_oas", "hy_oas", "baa10y"])).unwrap();
    let got: Vec<&str> = selected.iter().map(|l| l.key.as_str()).collect();
    assert_eq!(
        got,
        vec!["ig_oas", "hy_oas", "baa10y"],
        "message order must follow the config list, not analyze()'s coverage sort"
    );
}

#[test]
fn select_message_series_unknown_key_fails_by_name() {
    let series = vec![input(
        "baa10y",
        SeriesKind::Spread,
        Frequency::Daily,
        obs(&[("1986-01-02", 1.0), ("2026-07-24", 1.59)]),
    )];
    let lines = analyze(&series).unwrap();
    let err = select_message_series(&lines, &keys(&["nope"])).unwrap_err();
    assert!(
        err.message.contains("nope"),
        "error must name the unresolved key: {}",
        err.message
    );
}

#[test]
fn select_message_series_finds_the_derived_row_only_after_analyze() {
    // `select_message_series` takes `&[SeriesLine]` -- analyze()'s output
    // type, which is the only place `baa−aaa` is ever constructed. This is
    // structurally enforced (there is no overload taking `&[SeriesInput]`),
    // but this test also pins the behaviour: naming the derived key in
    // config must resolve once analyze() has run.
    let series = vec![
        input(
            "baa",
            SeriesKind::Yield,
            Frequency::Monthly,
            obs(&[("1919-01-01", 5.5), ("2026-06-01", 6.0)]),
        ),
        input(
            "aaa",
            SeriesKind::Yield,
            Frequency::Monthly,
            obs(&[("1919-01-01", 5.0), ("2026-06-01", 5.5)]),
        ),
    ];
    let lines = analyze(&series).unwrap();
    let selected = select_message_series(&lines, &keys(&[BAA_AAA_KEY])).unwrap();
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].key, BAA_AAA_KEY);
}

#[test]
fn format_message_selects_only_the_configured_series() {
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
            obs(&[("2023-07-28", 2.0), ("2026-07-24", 2.79)]),
        ),
    ];
    let msg = format_message(&series, "2026-07-31", &keys(&["baa10y"])).unwrap();
    assert!(msg.contains("[baa10y]"), "{msg}");
    assert!(!msg.contains("[hy_oas]"), "unlisted series must not render: {msg}");
}

// ── order ────────────────────────────────────────────────────────────────

#[test]
fn order_spreads_before_yields_longest_coverage_first() {
    // Config order deliberately puts short-coverage first and yields first;
    // analyze() must reorder: spreads block, then longest coverage first.
    // (This tests analyze() directly -- unaffected by message-series
    // selection, which is a later step.)
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
    let series = vec![input("hy_oas", SeriesKind::Spread, Frequency::Daily, rows)];
    let lines = analyze(&series).unwrap();
    assert_eq!(lines.len(), 1);
    let labels: Vec<&str> = lines[0].windows.iter().map(|w| w.label.as_str()).collect();
    assert_eq!(
        labels,
        vec!["近1年", "自2023"],
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
    assert!(
        msg.lines().any(|l| l.contains("近1年") && l.contains("筆比現在低")),
        "近1年 count must still appear: {msg}"
    );
    assert!(
        msg.lines().any(|l| l.contains("自2023") && l.contains("筆比現在低")),
        "full-history count must still appear: {msg}"
    );
    assert!(msg.contains("[hy_oas]"), "title line must still carry the key: {msg}");
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

#[test]
fn window_label_is_the_actual_start_year() {
    let msg = render_lines(&golden_lines(), "2026-07-31");
    assert!(!msg.contains("全庫"), "全庫 hides that each row is a different ruler");
    assert!(msg.contains("自1919") || msg.contains("自1986"), "got:\n{msg}");
}

#[test]
fn three_series_with_different_coverage_get_three_different_labels() {
    // The golden's three coverage years (1919/1986/2023) span both
    // frequencies; v3 has no split, so all are always rendered.
    let msg = render_lines(&golden_lines(), "2026-07-31");
    let years: std::collections::HashSet<&str> = msg
        .lines()
        .filter(|l| l.contains("筆比現在低"))
        .filter_map(|l| l.split_whitespace().find(|t| t.starts_with('自')))
        .collect();
    assert!(
        years.len() >= 3,
        "the golden spans 1919/1986/2023; each must print its own ruler, got {years:?}"
    );
}

/// Unlike the two tests above, this one supplies raw `Observation` rows and
/// goes through `analyze()` / `series_line_from_rows()` — the real path that
/// computes the label from data. A hand-authored label can never catch a
/// regression in that slicing (wrong bound, off-by-one, slicing the month
/// instead of the year); this test can.
#[test]
fn start_year_label_is_derived_from_the_data_not_supplied() {
    let series = vec![input(
        "baa10y",
        SeriesKind::Spread,
        Frequency::Daily,
        obs(&[("1986-01-02", 1.0), ("2026-07-31", 2.0)]),
    )];
    let lines = analyze(&series).expect("kind is set");
    let msg = render_lines(&lines, "2026-07-31");
    assert!(
        msg.contains("自1986"),
        "year must come from rows[0].date through year_str; got:\n{msg}"
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
fn precision_value_two_decimals_share_truncated_to_one_decimal() {
    let line = SeriesLine {
        key: "ig_oas".into(),
        label: "ig_oas".into(),
        kind: SeriesKind::Spread,
        value: Some(0.8),
        windows: vec![
            WindowPct { label: "近1年".into(), below: 150, n: 250 },
            WindowPct { label: "自2023".into(), below: 166, n: 750 },
        ],
        coverage_start: Some("2023-07-28".into()),
        latest: Some("2026-07-24".into()),
        frequency: Frequency::Daily,
        config_order: 0,
    };
    let msg = render_lines(&[line], "2026-07-31");
    assert!(msg.contains("0.80%"), "value must be two decimal places: {msg}");
    assert!(msg.contains("250 筆裡 150 筆比現在低(60.0%)"), "{msg}");
    assert!(msg.contains("750 筆裡 166 筆比現在低(22.1%)"), "{msg}");
    assert!(!msg.contains("0.800"), "{msg}");
}

// ── provenance ───────────────────────────────────────────────────────────

#[test]
fn provenance_is_coverage_year_not_fred_id() {
    let series = vec![input(
        "baa10y",
        SeriesKind::Spread,
        Frequency::Daily,
        obs(&[("1986-01-02", 1.0), ("2026-07-24", 1.59)]),
    )];
    // series_id is FRED_baa10y via helper — must not appear.
    let msg = format_message(&series, "2026-07-31", &keys(&["baa10y"])).unwrap();
    assert!(
        msg.contains("自1986"),
        "coverage start year required (the full-history window label): {msg}"
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
    assert!(msg.contains("月 至 2026-06"), "monthly latest required: {msg}");
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
    // 2026-06-01. The 資料 line must report monthly 至 2026-06 (the min),
    // never 2026-07 (the max of the inputs).
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
    let msg = format_message(&series, "2026-07-31", &keys(&["aaa", "baa"])).unwrap();
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
    let msg = format_message(&series, "2026-07-31", &keys(&["baa10y", "hy_oas", "aaa"])).unwrap();
    assert!(
        msg.contains("hy_oas") && msg.contains("n/a"),
        "missing series must render as n/a, not vanish: {msg}"
    );
    // Value now lives on the title line, so n/a sits right there.
    assert!(
        msg.contains("hy_oas  n/a   [hy_oas]"),
        "n/a must appear directly on the title line: {msg}"
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
    let err = format_message(&series, "2026-07-31", &keys(&["hy_oas"]))
        .expect_err("missing kind must err");
    assert!(
        err.message.contains("hy_oas") && err.message.contains("kind"),
        "error must name the series and the missing kind: {}",
        err.message
    );
}

// ── structural block split ───────────────────────────────────────────────

#[test]
fn spread_and_yield_blocks_are_separate_with_meaning_labels() {
    let msg = render_lines(&golden_lines(), "2026-07-31");
    let spread_hdr = msg
        .find("利差 —— 相對某個基準多出的殖利率")
        .expect("spread header");
    let yield_hdr = msg
        .find("總殖利率 —— 含利率在內的全部借款成本,與上一區不可互比")
        .expect("yield header");
    assert!(spread_hdr < yield_hdr, "spread block must precede yield block");
    // Keys land under the right header. Title lines now carry the value
    // between the label and `[key]`, so anchor on the bracketed key rather
    // than the old `Label [key]` adjacency.
    let baa10y = msg.find("[baa10y]").unwrap();
    let aaa = msg.find("[aaa]").expect("aaa title line");
    assert!(baa10y > spread_hdr && baa10y < yield_hdr, "baa10y under spreads");
    assert!(aaa > yield_hdr, "aaa under yields");
}

// ── SIGNAL-ONLY closer ───────────────────────────────────────────────────

#[test]
fn closes_with_signal_only_and_has_no_status_line() {
    let msg = render_lines(&golden_lines(), "2026-07-31");
    assert!(
        msg.contains(
            "SIGNAL-ONLY:窗口越短對當下越敏感,越長越穩定。它們回答不同的問題,不可跨列比。"
        ),
        "{msg}"
    );
    assert!(
        !msg.contains("狀態：") && !msg.contains("狀態:"),
        "cds-con deliberately has no 狀態 line: {msg}"
    );
}

// ── width bound ──────────────────────────────────────────────────────────

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

    fn check(parts: &[cds_con::render::Segment]) {
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

    // Short ASCII-label fixture (structural coverage: every block, both
    // frequencies, derived row).
    check(&render_parts(&golden_lines(), "2026-07-31"));
    // The real, longer Chinese labels -- this is the fixture that actually
    // exercises the width the bound was raised for (v3 §3 puts the value on
    // the title line). Without this, a regression in label length would only
    // be caught in production.
    check(&render_parts(&v3_golden_lines(), "2026-08-04"));
}
