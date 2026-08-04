//! Render-layer tests for cds-con.
//!
//! The golden message (owner-approved, real data from 2026-07-30) is the
//! oracle for exact shape. Every fixed rule (order, windows, precision,
//! units, provenance, freshness, missing series, missing kind, message-series
//! selection, the lead pair) has its own named test so a mutation gate can
//! point at a specific failure.
//!
//! **2026-08-04 lead-block redesign.** The message now opens with a guided
//! pair (`cds_message_lead`) -- the same Baa bonds with and without the
//! risk-free rate -- before the older per-series `──── 佐證 ────` block.
//! See `docs/specs/2026-08-04-cds-con-readability-v2-design.md` and
//! `src/render.rs`'s module doc comment for the full rationale.

use cds_con::render::{
    analyze, format_message, parse_message_lead, parse_message_series, render_lines,
    render_parts, resolve_lead, select_message_series, width_bound, Frequency, LeadEntry,
    LineKind, SeriesInput, SeriesLine, WindowPct, BAA_AAA_KEY, BAA_AAA_LABEL,
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

/// Build a two-entry lead from `lines`, keyed by `k0`/`k1`, displayed as
/// `l0`/`l1`. Panics (via `expect`) if either key is absent -- a test that
/// asks for a lead pair the fixture cannot supply is a broken test, not a
/// case to handle gracefully.
fn lead_pair<'a>(
    lines: &'a [SeriesLine],
    k0: &str,
    l0: &'a str,
    k1: &str,
    l1: &'a str,
) -> Vec<(&'a SeriesLine, &'a str)> {
    vec![
        (lines.iter().find(|l| l.key == k0).expect(k0), l0),
        (lines.iter().find(|l| l.key == k1).expect(k1), l1),
    ]
}

/// General-purpose 9-series fixture (labels are just the key -- these tests
/// exercise layout/ordering/freshness rules, not the live Chinese labels,
/// which are config, not Rust). Spans three coverage years (1919/1986/2023)
/// across both frequencies so order/window/freshness rules all have
/// something to bite on. Every series here always renders in `shown`;
/// individual tests decide which (if any) two become the lead pair.
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

/// The exact five series `cds_message_series` shows, with the real
/// 2026-07-30 values/counts the owner approved. Hand-authored SeriesLines,
/// not run through `analyze()` -- this is the layout oracle for
/// [`golden_message`], the byte-for-byte test. `baa10y` and `baa` are the
/// lead pair; `baa−aaa`, `hy_oas` and `ig_oas` are the 佐證 block.
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

/// The owner-approved lead labels for `v3_golden_lines()` -- prose about the
/// PAIRING (`cds_message_lead`'s own field), never `cds_series`' own
/// per-series `Label`.
const LEAD_SPREAD_LABEL: &str = "扣掉利率(利差)";
const LEAD_YIELD_LABEL: &str = "沒扣(總殖利率)";

fn golden_lead(lines: &[SeriesLine]) -> Vec<(&SeriesLine, &str)> {
    lead_pair(lines, "baa10y", LEAD_SPREAD_LABEL, "baa", LEAD_YIELD_LABEL)
}

const GOLDEN: &str = "💾 信用利差 · 2026-07-30\n%＝該窗口內,比今天更低的觀測比例\n\n同一批 Baa 公司債,一條扣掉利率、一條沒扣\n\n扣掉利率(利差)  1.63%\n  近1年 24.4%  近10年 12.6%  自1986 13.7%\n\n沒扣(總殖利率)  6.19%\n  近1年 92.3%  近10年 95.8%  自1919 50.5%\n\n兩條的差就是十年期美債\n所以下面那條高,可能是利率,不是公司快倒閉\n\n──── 佐證 ────\n\nBaa 比 Aaa 多出的殖利率  0.43%\n  近1年 13 筆裡 0 筆比現在低\n  近10年 121 筆裡 0 筆比現在低\n  自1919 1291 筆裡 22 筆比現在低(1.7%)\n\n高收益債相對基準多出的殖利率  2.84%\n  近1年 265 筆裡 117 筆比現在低(44.1%)\n  自2023 789 筆裡 194 筆比現在低(24.5%)\n\n投資級債相對基準多出的殖利率  0.80%\n  近1年 265 筆裡 155 筆比現在低(58.4%)\n  自2023 788 筆裡 173 筆比現在低(21.9%)\n\n資料:日 至 2026-07-30(5 天前)・月 至 2026-07\nSIGNAL-ONLY:窗口越短對當下越敏感,越長越穩定。";

// ── exact shape ──────────────────────────────────────────────────────────

#[test]
fn golden_message() {
    // as_of 2026-08-04 is 5 days after the daily latest (2026-07-30), matching
    // the freshness line's "(5 天前)" -- there is now only one message shape,
    // so only one golden.
    let lines = v3_golden_lines();
    let lead = golden_lead(&lines);
    let rendered = render_lines(&lines, &lead, "2026-08-04");
    assert_eq!(
        rendered, GOLDEN,
        "the lead-block message must match the owner-approved target byte-for-byte"
    );
}

#[test]
fn golden_message_also_reachable_through_format_message() {
    // The golden fixture is hand-authored SeriesLines; this test drives the
    // SAME shape through the real production pipeline (SeriesInput ->
    // analyze -> select_message_series -> resolve_lead -> render), using
    // raw config strings parsed by the real parsers, so a bug in any one
    // layer (not just render_lines' formatting) would be caught.
    let baa_aaa_inputs_baa = input(
        "baa",
        SeriesKind::Yield,
        Frequency::Monthly,
        obs(&[("1919-01-01", 5.50), ("2026-07-01", 6.19)]),
    );
    // Only two real observations aren't enough to reproduce the exact golden
    // counts (those depend on the full historical series), so this test
    // checks structure and presence, not byte-for-byte equality with GOLDEN.
    let series = vec![
        baa_aaa_inputs_baa,
        input(
            "baa10y",
            SeriesKind::Spread,
            Frequency::Daily,
            obs(&[("1986-01-02", 1.0), ("2026-07-30", 1.63)]),
        ),
    ];
    let message_keys = parse_message_series("baa10y,baa").unwrap();
    let lead_config = parse_message_lead("baa10y|扣掉利率(利差);baa|沒扣(總殖利率)").unwrap();
    let msg = format_message(&series, "2026-08-04", &message_keys, &lead_config).unwrap();
    assert!(msg.contains("扣掉利率(利差)  1.63%"), "{msg}");
    assert!(msg.contains("沒扣(總殖利率)  6.19%"), "{msg}");
    assert!(msg.contains("兩條的差就是十年期美債"), "{msg}");
}

// ── counts, share, truncation (佐證 block) ─────────────────────────────

#[test]
fn wording_is_strictly_below_never_at_most() {
    let msg = render_lines(&golden_lines(), &[], "2026-07-31");
    assert!(msg.contains("筆比現在低"), "must state 低於 in some form");
    assert!(!msg.contains("不高於"), "不高於 is <=, the implementation is <");
}

#[test]
fn zero_below_renders_as_zero_count_with_no_parenthetical_share() {
    // A series sitting at its window minimum must print `N 筆裡 0 筆比現在低`,
    // never a blank, an omitted window, or a dash -- the p0-ambiguity fix
    // carried from v2/v3. The lead-block redesign additionally DROPS the
    // `(0.0%)` parenthetical for this case: a bare `0.0%` cannot tell a
    // reader "exactly zero" from "truncated down from something small", but
    // the count sitting right there can. This is a deliberate asymmetry
    // (non-zero windows keep the parenthetical) -- see
    // `share_percent_is_truncated_never_rounded_up` below.
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
    let msg = render_lines(&[line], &[], "2026-07-31");
    assert!(msg.contains("13 筆裡 0 筆比現在低"), "got:\n{msg}");
    assert!(
        !msg.contains("(0.0%)"),
        "zero-below must never print a parenthetical share: {msg}"
    );
}

#[test]
fn share_percent_is_truncated_never_rounded_up() {
    // 2/3 = 66.666...% -- standard 1-decimal rounding gives 66.7%, but the
    // required behaviour is truncation, so the correct printed share is
    // 66.6%. below > 0 here, so the parenthetical still appears.
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
    let msg = render_lines(&[line], &[], "2026-07-31");
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
    let msg = render_lines(&[line], &[], "2026-07-31");
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
            .all(|l| l.matches('%').count() <= 1),
        "a 佐證 window line carries at most one %, the share: {msg}"
    );
}

// ── rate and share never share a line ───────────────────────────────────

#[test]
fn rate_and_share_never_share_a_line_in_the_supporting_block() {
    // The 佐證 block is unchanged in shape from before the lead-block
    // redesign: the value's `%` (a rate) sits on the title line, and a
    // window's `%` (a share) sits on its own line below -- never both on one
    // line, so a rate and a share can never collide as two meanings of one
    // symbol.
    let msg = render_lines(&golden_lines(), &[], "2026-07-31");
    let mut saw_rate_line = false;
    let mut saw_share_line = false;
    for line in msg.lines() {
        let pct_count = line.matches('%').count();
        assert!(
            pct_count <= 1,
            "in the 佐證 block a rate and a share must never appear on the same line: {line}"
        );
        if pct_count == 1 && !line.starts_with(' ') {
            saw_rate_line = true;
        }
        if pct_count == 1 && line.contains("筆比現在低") {
            saw_share_line = true;
        }
    }
    assert!(saw_rate_line, "expected at least one title (rate) line: {msg}");
    assert!(saw_share_line, "expected at least one window (share) line: {msg}");
}

#[test]
fn lead_title_and_windows_are_always_separate_lines() {
    // The lead block compresses ALL windows onto one line, which
    // reintroduces multiple `%` per line (24.4%, 12.6%, 13.7% are all
    // shares) -- but the collision the original rule guarded against was a
    // RATE's `%` (the title's `1.63%`) sitting on the SAME line as a
    // share's `%`. That must still never happen: the title line carries
    // exactly the rate, the windows line carries only shares.
    let lines = v3_golden_lines();
    let lead = golden_lead(&lines);
    let msg = render_lines(&lines, &lead, "2026-08-04");
    for title in [LEAD_SPREAD_LABEL, LEAD_YIELD_LABEL] {
        let title_line = msg
            .lines()
            .find(|l| l.starts_with(title))
            .unwrap_or_else(|| panic!("expected a title line starting with {title}: {msg}"));
        assert_eq!(
            title_line.matches('%').count(),
            1,
            "lead title line must carry exactly the rate: {title_line}"
        );
        assert!(
            !title_line.contains("筆比現在低") && !title_line.contains("近1年"),
            "lead title line must not also carry window content: {title_line}"
        );
    }
    // The compressed windows line, conversely, never carries the rate.
    let windows_line = msg
        .lines()
        .find(|l| l.contains("近1年") && l.contains("自1986"))
        .expect("expected the baa10y compressed windows line");
    assert!(
        !windows_line.contains("1.63%"),
        "windows line must never carry the title's rate: {windows_line}"
    );
}

// ── block headers / lead prose ──────────────────────────────────────────

#[test]
fn lead_block_carries_fixed_explanatory_prose() {
    // Fixed prose about the pairing, not a read of today's market shape --
    // these strings never interpolate a live number, so they cannot become
    // false on a day the market moves.
    let msg = render_lines(&golden_lines(), &[], "2026-07-31");
    assert!(msg.contains("%＝該窗口內,比今天更低的觀測比例"));
    assert!(msg.contains("同一批 Baa 公司債,一條扣掉利率、一條沒扣"));
    assert!(msg.contains("兩條的差就是十年期美債"));
    assert!(msg.contains("所以下面那條高,可能是利率,不是公司快倒閉"));
}

#[test]
fn spreads_never_claim_to_be_the_price_of_credit_risk() {
    let msg = render_lines(&golden_lines(), &[], "2026-07-31");
    assert!(!msg.contains("信用風險本身的價格"));
}

// ── header date ──────────────────────────────────────────────────────────

#[test]
fn header_carries_the_most_recent_daily_date() {
    // golden_lines()'s daily series all latest at 2026-07-24; the header is a
    // fact about the data ("what date do these numbers reflect"), not the
    // run date.
    let msg = render_lines(&golden_lines(), &[], "2026-07-31");
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
    let msg = render_lines(&[line], &[], "2026-07-31");
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
    // loudly rather than silently skip it.
    let err = parse_message_series("baa10y,,hy_oas").unwrap_err();
    assert!(err.contains("empty") && err.contains("position 2"), "{err}");
    let err = parse_message_series("baa10y,hy_oas,").unwrap_err();
    assert!(err.contains("empty"), "{err}");
}

// ── message-lead config: parsing ────────────────────────────────────────

#[test]
fn message_lead_parses_two_key_label_records_in_order() {
    let got = parse_message_lead("baa10y|扣掉利率(利差);baa|沒扣(總殖利率)").unwrap();
    assert_eq!(
        got,
        vec![
            LeadEntry { key: "baa10y".into(), label: "扣掉利率(利差)".into() },
            LeadEntry { key: "baa".into(), label: "沒扣(總殖利率)".into() },
        ]
    );
}

#[test]
fn message_lead_trims_whitespace_around_fields() {
    let got = parse_message_lead(" baa10y | 扣掉利率 ; baa | 沒扣 ").unwrap();
    assert_eq!(got[0].key, "baa10y");
    assert_eq!(got[0].label, "扣掉利率");
    assert_eq!(got[1].key, "baa");
    assert_eq!(got[1].label, "沒扣");
}

#[test]
fn message_lead_empty_value_fails() {
    let err = parse_message_lead("").unwrap_err();
    assert!(err.contains("empty"), "{err}");
    let err = parse_message_lead("   ").unwrap_err();
    assert!(err.contains("empty"), "{err}");
}

#[test]
fn message_lead_wrong_field_count_fails() {
    let err = parse_message_lead("baa10y;baa|沒扣").unwrap_err();
    assert!(err.contains("key|Label"), "{err}");
    let err = parse_message_lead("baa10y|a|b;baa|c").unwrap_err();
    assert!(err.contains("key|Label"), "{err}");
}

#[test]
fn message_lead_wrong_entry_count_fails() {
    // Not zero, not one, not three -- exactly two, since it is a pair by
    // construction, not an open-ended list.
    let err = parse_message_lead("baa10y|A").unwrap_err();
    assert!(err.contains("exactly two"), "{err}");
    let err = parse_message_lead("baa10y|A;baa|B;hy_oas|C").unwrap_err();
    assert!(err.contains("exactly two"), "{err}");
}

#[test]
fn message_lead_duplicate_key_fails() {
    let err = parse_message_lead("baa10y|A;baa10y|B").unwrap_err();
    assert!(err.contains("duplicate") && err.contains("baa10y"), "{err}");
}

// ── message-lead config: resolution ─────────────────────────────────────

#[test]
fn resolve_lead_finds_series_by_key_in_config_order() {
    let series = vec![
        input(
            "baa10y",
            SeriesKind::Spread,
            Frequency::Daily,
            obs(&[("1986-01-02", 1.0), ("2026-07-24", 1.59)]),
        ),
        input(
            "baa",
            SeriesKind::Yield,
            Frequency::Monthly,
            obs(&[("1919-01-01", 5.5), ("2026-06-01", 6.0)]),
        ),
    ];
    let lines = analyze(&series).unwrap();
    let shown = select_message_series(&lines, &keys(&["baa10y", "baa"])).unwrap();
    let lead_config = parse_message_lead("baa|沒扣;baa10y|扣掉").unwrap();
    let resolved = resolve_lead(&shown, &lead_config).unwrap();
    // Order follows lead_config, not shown's (analyze/select) order.
    assert_eq!(resolved[0].0.key, "baa");
    assert_eq!(resolved[0].1, "沒扣");
    assert_eq!(resolved[1].0.key, "baa10y");
    assert_eq!(resolved[1].1, "扣掉");
}

#[test]
fn resolve_lead_unknown_key_fails_by_name() {
    let series = vec![input(
        "baa10y",
        SeriesKind::Spread,
        Frequency::Daily,
        obs(&[("1986-01-02", 1.0), ("2026-07-24", 1.59)]),
    )];
    let lines = analyze(&series).unwrap();
    let shown = select_message_series(&lines, &keys(&["baa10y"])).unwrap();
    let lead_config = parse_message_lead("baa10y|A;doesnotexist|B").unwrap();
    let err = resolve_lead(&shown, &lead_config).unwrap_err();
    assert!(
        err.message.contains("doesnotexist"),
        "error must name the unresolved key: {}",
        err.message
    );
}

#[test]
fn format_message_fails_when_lead_names_a_series_absent_from_message_series() {
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
    // hy_oas is a real series but is NOT in cds_message_series -- the lead
    // must resolve against `shown` (post-selection), not the raw series list.
    let lead_config = parse_message_lead("baa10y|A;hy_oas|B").unwrap();
    let err = format_message(&series, "2026-07-31", &keys(&["baa10y"]), &lead_config)
        .expect_err("lead naming a series outside cds_message_series must fail");
    assert!(err.message.contains("hy_oas"), "{}", err.message);
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
        input(
            "ig_oas",
            SeriesKind::Spread,
            Frequency::Daily,
            obs(&[("2023-07-28", 0.5), ("2026-07-24", 0.80)]),
        ),
    ];
    let lead_config = parse_message_lead("baa10y|扣掉利率(利差);hy_oas|沒扣(總殖利率)").unwrap();
    let msg = format_message(
        &series,
        "2026-07-31",
        &keys(&["baa10y", "hy_oas"]),
        &lead_config,
    )
    .unwrap();
    assert!(msg.contains("扣掉利率(利差)"), "{msg}");
    assert!(msg.contains("沒扣(總殖利率)"), "{msg}");
    assert!(!msg.contains("ig_oas"), "unlisted series must not render: {msg}");
}

// ── order ────────────────────────────────────────────────────────────────

#[test]
fn order_spreads_before_yields_longest_coverage_first() {
    // Config order deliberately puts short-coverage first and yields first;
    // analyze() must reorder: spreads block, then longest coverage first.
    // (This tests analyze() directly -- unaffected by message-series
    // selection or the lead pair, which are later steps.)
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

    let msg = render_lines(&lines, &[], "2026-07-31");
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
    assert!(msg.contains("hy_oas"), "title line must still carry the label: {msg}");
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
    let msg = render_lines(&golden_lines(), &[], "2026-07-31");
    assert!(!msg.contains("全庫"), "全庫 hides that each row is a different ruler");
    assert!(msg.contains("自1919") || msg.contains("自1986"), "got:\n{msg}");
}

#[test]
fn three_series_with_different_coverage_get_three_different_labels() {
    // The golden's three coverage years (1919/1986/2023) span both
    // frequencies; every series in this fixture is always rendered (in the
    // 佐證 block, since lead is empty here).
    let msg = render_lines(&golden_lines(), &[], "2026-07-31");
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
    let msg = render_lines(&lines, &[], "2026-07-31");
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
    let msg = render_lines(&[line], &[], "2026-07-31");
    assert!(msg.contains("0.80%"), "value must be two decimal places: {msg}");
    assert!(msg.contains("250 筆裡 150 筆比現在低(60.0%)"), "{msg}");
    assert!(msg.contains("750 筆裡 166 筆比現在低(22.1%)"), "{msg}");
    assert!(!msg.contains("0.800"), "{msg}");
}

// ── provenance ───────────────────────────────────────────────────────────

#[test]
fn provenance_is_coverage_year_not_fred_id() {
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
    let lead_config = parse_message_lead("baa10y|扣掉利率;hy_oas|沒扣").unwrap();
    // series_id is FRED_baa10y via helper — must not appear.
    let msg = format_message(
        &series,
        "2026-07-31",
        &keys(&["baa10y", "hy_oas"]),
        &lead_config,
    )
    .unwrap();
    assert!(
        msg.contains("自1986"),
        "coverage start year required (the full-history window label, now \
         inside baa10y's compressed lead windows line): {msg}"
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
    let msg = render_lines(&golden_lines(), &[], "2026-07-31");
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
    let msg = format_message(&series, "2026-07-31", &keys(&["aaa", "baa"]), &[]).unwrap();
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
    // aaa alone in the lead (single-entry lead is legal when calling
    // format_message directly -- only the parsed config string enforces
    // "exactly two") pulls the sole yield out of 佐證, leaving 佐證 as
    // baa10y + hy_oas, both spreads, so require_single_kind passes.
    let lead_config = vec![LeadEntry { key: "aaa".into(), label: "AAA-LEAD".into() }];
    let msg = format_message(
        &series,
        "2026-07-31",
        &keys(&["baa10y", "hy_oas", "aaa"]),
        &lead_config,
    )
    .unwrap();
    assert!(
        msg.contains("hy_oas") && msg.contains("n/a"),
        "missing series must render as n/a, not vanish: {msg}"
    );
    assert!(
        msg.contains("hy_oas  n/a"),
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
    let err = format_message(&series, "2026-07-31", &keys(&["hy_oas"]), &[])
        .expect_err("missing kind must err");
    assert!(
        err.message.contains("hy_oas") && err.message.contains("kind"),
        "error must name the series and the missing kind: {}",
        err.message
    );
}

// ── structural block split (v4 item 4) ──────────────────────────────────

#[test]
fn spread_and_yield_adjacency_is_confined_to_the_lead_block() {
    // v4 item 4: a spread and a yield may only appear adjacent inside the
    // lead block, and the lead block must carry the explanation line. The
    // lead block is the ONE place a spread/yield pair is allowed to sit
    // next to each other, and it is safe there only because
    // `兩條的差就是十年期美債` explains why they differ.
    let lines = golden_lines();
    let lead = golden_lead(&lines);
    let msg = render_lines(&lines, &lead, "2026-07-31");
    let spread_title = msg
        .lines()
        .find(|l| l.starts_with(LEAD_SPREAD_LABEL))
        .expect("lead spread title");
    let explanation = msg.find("兩條的差就是十年期美債").expect("explanation line");
    let spread_pos = msg.find(spread_title).unwrap();
    assert!(
        spread_pos < explanation,
        "lead block must precede its own explanation: {msg}"
    );
    assert!(msg.contains("所以下面那條高,可能是利率,不是公司快倒閉"), "{msg}");
}

#[test]
fn mixed_kind_outside_the_lead_pair_fails_the_run() {
    // Same idea, but proving the NEGATIVE guarantee: with an empty lead,
    // every configured series lands in 佐證, mixing a spread (baa10y) and a
    // yield (aaa). 佐證 has no per-kind header left (that is what the
    // lead-block redesign removed), so this must fail loudly rather than
    // silently render a spread and a yield adjacent with no explanation --
    // exactly the defect the old split existed to prevent.
    let series = vec![
        input(
            "baa10y",
            SeriesKind::Spread,
            Frequency::Daily,
            obs(&[("1986-01-02", 1.0), ("2026-07-24", 1.59)]),
        ),
        input(
            "aaa",
            SeriesKind::Yield,
            Frequency::Monthly,
            obs(&[("1919-01-01", 5.0), ("2026-06-01", 5.5)]),
        ),
    ];
    let err = format_message(&series, "2026-07-31", &keys(&["baa10y", "aaa"]), &[])
        .expect_err("mixed-kind 佐證 must fail the run");
    assert!(
        err.message.contains("spread") && err.message.contains("yield"),
        "error should name both kinds: {}",
        err.message
    );
}

#[test]
fn lead_pair_of_matching_kinds_is_not_rejected() {
    // The lead pair itself is NOT required to be one spread and one yield by
    // Rust -- that correctness is the operator's config responsibility, the
    // same way a wrong `cds_series.Label` is (see design doc's "regressions
    // this suite would still pass"). Two spreads in the lead must still
    // render, not error.
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
    let lead_config = parse_message_lead("baa10y|A;hy_oas|B").unwrap();
    let msg = format_message(
        &series,
        "2026-07-31",
        &keys(&["baa10y", "hy_oas"]),
        &lead_config,
    )
    .expect("two spreads in the lead pair is not an error");
    assert!(msg.contains('A') && msg.contains('B'), "{msg}");
}

// ── SIGNAL-ONLY closer ───────────────────────────────────────────────────

#[test]
fn closes_with_signal_only_and_has_no_status_line() {
    let msg = render_lines(&golden_lines(), &[], "2026-07-31");
    assert!(
        msg.contains("SIGNAL-ONLY:窗口越短對當下越敏感,越長越穩定。"),
        "{msg}"
    );
    assert!(
        !msg.contains("狀態：") && !msg.contains("狀態:"),
        "cds-con deliberately has no 狀態 line: {msg}"
    );
}

// ── width bound ──────────────────────────────────────────────────────────

/// Display-column width under the CJK-is-2 model the width bound assumes.
/// Kept as an independent copy from `src/render.rs`'s `display_width` (not
/// imported) so this test verifies the renderer's split decision against its
/// OWN arithmetic, rather than the two sides trusting the same possibly-wrong
/// model.
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

#[test]
fn every_rendered_line_fits_its_width_bound() {
    // Proxy, not proof: the transport (`parse_mode: None`) renders a
    // proportional font, where this CJK-is-2-columns model does not describe
    // the real wrap point. What this prevents is a line bloating back to a
    // size that breaks even a monospace reader; it is not a guarantee
    // against wrapping on a phone.

    // Short ASCII-label fixture (structural coverage: 佐證 block, both
    // frequencies, derived row) with no lead -- everything lands in 佐證.
    check(&render_parts(&golden_lines(), &[], "2026-07-31"));
    // The real, longer Chinese labels with the real lead pair -- this is
    // the fixture that actually exercises the widths measured for the
    // lead-block redesign (the compressed windows line is the widest line
    // the new shape produces, at 41 columns; see `src/render.rs`'s
    // `WIDTH_BOUND` doc comment).
    let lines = v3_golden_lines();
    let lead = golden_lead(&lines);
    check(&render_parts(&lines, &lead, "2026-08-04"));
}

/// A series-line fixture shaped exactly like the real `hy_oas` row (same
/// value, windows, coverage, key) except for the label -- lets a test drive
/// an absurdly long label through the renderer without touching any other
/// dimension of the layout.
fn line_with_label(label: &str) -> SeriesLine {
    SeriesLine {
        key: "hy_oas".into(),
        label: label.to_string(),
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
    }
}

/// Swap the `hy_oas` slot in the real fixture for `label`, keeping every
/// other series (and therefore the headers, lead block, freshness line and
/// footer) exactly as the live message renders them -- so the assertion
/// below exercises the FULL message shape, not an isolated single-series
/// snippet. `hy_oas` sits in the 佐證 block (index 2, not part of the lead).
fn lines_with_hy_oas_label(label: &str) -> Vec<SeriesLine> {
    let mut lines = v3_golden_lines();
    lines[2] = line_with_label(label);
    lines
}

/// Asserts the renderer's actual guarantee for an overlong label: every
/// `Data` line EXCEPT the label's own line fits [`width_bound`]. Headers,
/// the pairing prose, the explanation lines, the `──── 佐證 ────` separator,
/// the freshness line and the footer are `LineKind::Prose` (see its doc
/// comment -- "allowed to wrap on a phone") and are present in `parts` (the
/// fixture is the full message, not an isolated snippet) but deliberately
/// out of scope for this bound, same as `every_rendered_line_fits_its_width_bound`
/// already treats them -- the `LineKind::Data` filter below is what skips
/// them, same mechanism as `check`.
///
/// This is the one case `title_lines` cannot fix: a label that alone
/// exceeds the bound still overflows on its own line, because the renderer
/// must never truncate a configured label to force a layout -- discarding
/// real `cds_series` data would be worse than one wrapped line. The label's
/// own line is therefore the single line this helper does not check.
fn assert_only_the_label_line_may_overflow(parts: &[cds_con::render::Segment], label: &str) {
    let mut saw_label_alone = false;
    for seg in parts.iter().filter(|s| s.kind == LineKind::Data) {
        if seg.text == label {
            saw_label_alone = true;
        } else {
            assert!(
                width(&seg.text) <= width_bound(),
                "line other than the overlong label itself must fit the bound \
                 ({} cols, bound {}): {}",
                width(&seg.text),
                width_bound(),
                seg.text
            );
        }
    }
    assert!(
        saw_label_alone,
        "expected the overlong label to render on its own line"
    );
}

#[test]
fn overlong_ascii_label_splits_the_title_line() {
    let label = "x".repeat(200);
    let lines = lines_with_hy_oas_label(&label);
    let lead = golden_lead(&lines);
    let parts = render_parts(&lines, &lead, "2026-08-04");
    assert_only_the_label_line_may_overflow(&parts, &label);

    // The split actually happened: the value line is a SEPARATE segment
    // from the label, not the label with the value appended to it.
    let value_line = parts
        .iter()
        .filter(|s| s.kind == LineKind::Data)
        .find(|s| s.text.starts_with("  2.84%"))
        .expect("expected a value line indented like a window row");
    assert!(
        !value_line.text.contains(&label),
        "value must no longer share the label's line: {}",
        value_line.text
    );
}

#[test]
fn overlong_cjk_label_splits_the_title_line() {
    // 200 CJK characters -> 400 display columns under the CJK-is-2 model:
    // the case WIDTH_BOUND's doc comment specifically calls out, since the
    // real `cds_series` labels are themselves CJK.
    let label = "測".repeat(200);
    let lines = lines_with_hy_oas_label(&label);
    let lead = golden_lead(&lines);
    let parts = render_parts(&lines, &lead, "2026-08-04");
    assert_only_the_label_line_may_overflow(&parts, &label);

    let value_line = parts
        .iter()
        .filter(|s| s.kind == LineKind::Data)
        .find(|s| s.text.starts_with("  2.84%"))
        .expect("expected a value line indented like a window row");
    assert!(
        !value_line.text.contains(&label),
        "value must no longer share the label's line: {}",
        value_line.text
    );
}

#[test]
fn todays_real_labels_never_split_and_golden_is_unchanged() {
    // The widest real line today is 41 columns -- well under the 48 bound --
    // so nothing should split. golden_message already pins byte-identical
    // output; this test additionally pins the STRUCTURAL signature of "no
    // split occurred" (no line equal to a bare label) so a future change
    // that started splitting real config without changing the golden text
    // would still be caught.
    let lines = v3_golden_lines();
    let lead = golden_lead(&lines);
    let rendered = render_lines(&lines, &lead, "2026-08-04");
    assert_eq!(
        rendered, GOLDEN,
        "today's real labels must not change the golden"
    );
    for line in &lines {
        assert!(
            !rendered.lines().any(|l| l == line.label),
            "label '{}' must not render alone on its own line -- nothing \
             should split today: {}",
            line.label,
            rendered
        );
    }
}
