//! The attribute-2 message: what it says, and what it must not say.
//!
//! The reading itself is pinned in `tests/cost.rs` against finance-cli's rule.
//! These pin the message built from it.

use cds_con::cost::Level;
use cds_con::render::{
    format_cost_variants, render_cost_lines, render_cost_parts, width_bound, Frequency, LineKind,
    SeriesInput,
};
use credit_store::{Observation, SeriesKind, SeriesSpec};

const AS_OF: &str = "2026-08-13";

fn baa_input(rows: &[(&str, f64)]) -> SeriesInput {
    SeriesInput {
        spec: SeriesSpec {
            key: "baa".into(),
            series_id: "BAA".into(),
            label: "Baa 公司債殖利率".into(),
            kind: Some(SeriesKind::Yield),
        },
        rows: rows
            .iter()
            .map(|(d, v)| Observation { date: (*d).to_string(), value: *v })
            .collect(),
        frequency: Frequency::Monthly,
    }
}

/// Three months, newest highest — two of three below it, so 66 after the
/// truncating divide `cost level` uses.
fn sample() -> SeriesInput {
    baa_input(&[("2026-05-01", 5.30), ("2026-06-01", 5.35), ("2026-07-01", 6.19)])
}

fn rendered(series: &SeriesInput, level: Option<&Level>) -> String {
    let lines = cds_con::render::analyze(std::slice::from_ref(series)).expect("analyze");
    render_cost_lines(&lines[0], level, AS_OF)
}

fn level(pct: usize, n: usize) -> Level {
    Level { date: "2026-07-01".into(), value: 6.19, pct, n }
}

// ── what the message says ────────────────────────────────────────────────────

#[test]
fn the_message_leads_with_the_reading_and_names_its_basis() {
    let out = rendered(&sample(), Some(&level(50, 1291)));
    let lines: Vec<&str> = out.lines().collect();

    assert_eq!(lines[0], "💾 企業債成本｜attribute 2");
    assert!(lines[2].starts_with("Baa 公司債殖利率"), "{out}");
    assert!(lines[2].contains("6.19%"), "{out}");
    // The label is meaningless without its basis and its n: the same number
    // reads differently on a window of 129 and one of 1291.
    assert_eq!(lines[3], "狀態：高（as-of 分位 50，n=1291）");
}

#[test]
fn the_median_cut_shows_through_to_the_rendered_label() {
    assert!(rendered(&sample(), Some(&level(50, 1291))).contains("狀態：高"));
    assert!(rendered(&sample(), Some(&level(49, 1291))).contains("狀態：不高"));
}

#[test]
fn a_month_the_series_does_not_reach_says_so_rather_than_reading_zero() {
    // 無資料 and "0th percentile" are opposite findings -- the cheapest
    // borrowing on record versus no observation at all.
    let out = rendered(&sample(), None);
    assert!(out.contains("狀態：無資料"), "{out}");
    assert!(!out.contains("分位 0"), "{out}");
}

#[test]
fn the_mechanism_prose_names_the_yield_the_as_of_basis_and_the_cut() {
    let out = rendered(&sample(), Some(&level(50, 1291)));
    assert!(out.contains("量的是殖利率本身,不是任何相減後的利差"), "{out}");
    assert!(out.contains("分位只用該月之前(含當月)的觀測,不回望未來"), "{out}");
    assert!(out.contains("切點是中位數,分位 ≥ 50 為「高」"), "{out}");
}

#[test]
fn the_footer_names_finance_cli_as_the_authority() {
    // The rule lives in finance-cli; this message follows it. Saying so in the
    // message is what stops a reader treating this skill as the definition.
    assert!(rendered(&sample(), Some(&level(50, 1291))).contains("finance-cli `cost level`"));
}

// ── what it must not say ─────────────────────────────────────────────────────

#[test]
fn no_spread_survives_anywhere_in_the_message() {
    // Attribute 2 is the yield itself. A spread in this message would be the
    // measure it replaced, back under a label that now means something else.
    let out = rendered(&sample(), Some(&level(50, 1291)));
    for banned in ["利差在", "扣掉利率", "沒扣", "baa−aaa", "Baa 比 Aaa"] {
        assert!(!out.contains(banned), "spread wording leaked: {banned}\n{out}");
    }
}

#[test]
fn the_trailing_windows_carry_no_label_of_their_own() {
    // They are a different basis from the as-of reading. Labelling each would
    // put several verdicts on one number -- the objection that used to rule
    // out labelling anything at all.
    let out = rendered(&sample(), Some(&level(50, 1291)));
    assert_eq!(out.matches("狀態：").count(), 1, "{out}");
    assert!(
        out.contains("這幾個窗口不是判定的依據,判定只看上面那個 as-of 分位"),
        "{out}"
    );
}

// ── structure ────────────────────────────────────────────────────────────────

#[test]
fn the_evidence_block_is_tagged_from_the_separator_to_its_last_line() {
    let lines = cds_con::render::analyze(std::slice::from_ref(&sample())).expect("analyze");
    let parts = render_cost_parts(&lines[0], Some(&level(50, 1291)), AS_OF);

    let tagged: Vec<&str> = parts.iter().filter(|p| p.evidence).map(|p| p.text.as_str()).collect();
    assert!(tagged.first().is_some_and(|t| t.contains("佐證")), "{tagged:?}");
    // The reading and its basis must stay OUT of the collapsed block: a
    // verdict behind a tap is a verdict the reader never sees.
    for p in parts.iter().filter(|p| p.evidence) {
        assert!(!p.text.contains("狀態："), "the reading must not collapse: {}", p.text);
    }
    // And the tag is one contiguous run -- flatten_html wraps a single range.
    let idx: Vec<usize> =
        parts.iter().enumerate().filter(|(_, p)| p.evidence).map(|(i, _)| i).collect();
    assert!(idx.windows(2).all(|w| w[1] == w[0] + 1), "evidence tag is not contiguous: {idx:?}");
}

#[test]
fn every_data_line_fits_the_width_bound() {
    let lines = cds_con::render::analyze(std::slice::from_ref(&sample())).expect("analyze");
    let parts = render_cost_parts(&lines[0], Some(&level(50, 1291)), AS_OF);
    for p in parts.iter().filter(|p| p.kind == LineKind::Data) {
        let w: usize = p
            .text
            .chars()
            .map(|c| if (c as u32) > 0x1100 { 2 } else { 1 })
            .sum();
        assert!(w <= width_bound(), "data line too wide ({w}): {}", p.text);
    }
}

#[test]
fn the_html_variant_collapses_the_evidence_and_the_plain_one_stays_bare() {
    let v = format_cost_variants(&sample(), Some(&level(50, 1291)), AS_OF).expect("variants");
    assert!(v.html.contains("<blockquote expandable>"), "{}", v.html);
    assert!(!v.plain.contains("<blockquote"), "{}", v.plain);
    // Same content, different markup: the reading is in both.
    assert!(v.plain.contains("狀態：高") && v.html.contains("狀態：高"));
}

#[test]
fn a_series_with_no_rows_still_renders_a_message_rather_than_erroring() {
    // run.rs refuses this case earlier (no usable observations is a failed
    // run), but the renderer must not panic if it ever gets here.
    let out = rendered(&baa_input(&[]), None);
    assert!(out.contains("狀態：無資料"), "{out}");
}
