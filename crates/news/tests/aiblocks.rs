//! Block parsing, theme layout, and the cross-batch dedup vote.
//!
//! These three share one representation: the rendered AI lines parsed back
//! into stories. Both consumers fail closed on a parse they do not recognise,
//! because a partial parse would silently drop a story during regrouping.

use news::crossdedup::{apply_cross_dedup, components, pair_votes, parse_cross_dedup_response, survivor, Group};
use news::render::{PAYWALL_CONT_PREFIX, PAYWALL_NOTE};
use news::theme::{parse_ai_blocks, strip_bullet_text, parse_theme_response, theme_layout_plan, theme_render};
use std::collections::BTreeMap;

const L: &str = "https://n/1";

fn lines(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

// ── block parsing ────────────────────────────────────────────────────────────

#[test]
fn a_bullet_becomes_one_block() {
    let b = parse_ai_blocks(&lines(&[&format!("- 甲 [🔗]({L})"), "- 乙"])).unwrap();
    assert_eq!(b.len(), 2);
    assert_eq!(b[0].headline, "甲");
    assert_eq!(b[0].access, "normal");
    assert_eq!((b[0].start, b[0].end), (0, 1));
}

#[test]
fn a_paywall_pair_is_one_block_spanning_two_lines() {
    let b = parse_ai_blocks(&lines(&[
        &format!("- 免費替代 [🔗]({L})"),
        &format!("{PAYWALL_CONT_PREFIX}原文：付費標題 [🔗]({L})  {PAYWALL_NOTE}"),
    ]))
    .unwrap();
    assert_eq!(b.len(), 1);
    assert_eq!(b[0].headline, "免費替代");
    assert_eq!(b[0].original_headline.as_deref(), Some("付費標題"));
    assert_eq!(b[0].access, "free_replacement");
    assert_eq!((b[0].start, b[0].end), (0, 2));
}

#[test]
fn a_single_paywalled_bullet_is_marked_paywalled() {
    let b = parse_ai_blocks(&lines(&[&format!("- 付費單則 [🔗]({L})  {PAYWALL_NOTE}")])).unwrap();
    assert_eq!(b[0].access, "paywalled");
}

#[test]
fn an_orphan_continuation_fails_the_whole_parse() {
    // No parent bullet consumed it, so the render drifted and regrouping would
    // lose a line.
    assert!(parse_ai_blocks(&lines(&[&format!("{PAYWALL_CONT_PREFIX}孤兒續行")])).is_none());
}

#[test]
fn a_line_that_is_neither_bullet_nor_blank_fails_the_parse() {
    assert!(parse_ai_blocks(&lines(&["不是條列"])).is_none());
}

#[test]
fn blank_separators_are_skipped_without_becoming_blocks() {
    let b = parse_ai_blocks(&lines(&["", "- 甲", "", "- 乙"])).unwrap();
    assert_eq!(b.len(), 2);
    // The slices no longer cover every physical line, which is what the
    // no-drop guard in the renderer checks for.
    assert!(b.iter().map(|x| x.end - x.start).sum::<usize>() < 4);
}

#[test]
fn a_bullet_is_reduced_to_its_headline() {
    assert_eq!(strip_bullet_text(&format!("- 甲 [🔗]({L})  {PAYWALL_NOTE}")), "甲");
    assert_eq!(strip_bullet_text("- 甲 [🔗](x) 尾巴"), "甲");
    assert_eq!(strip_bullet_text(&format!("- 甲 {PAYWALL_NOTE}")), "甲");
}

// ── theme labels ─────────────────────────────────────────────────────────────

const LABELS_OK: &str = r#"{"labels":[{"id":1,"theme":"產品發布"},{"id":2,"theme":"政策監管"}]}"#;

#[test]
fn a_complete_label_set_parses() {
    let m = parse_theme_response(LABELS_OK, 2).unwrap();
    assert_eq!(m[&1], "產品發布");
    assert_eq!(m[&2], "政策監管");
}

#[test]
fn a_json_fence_around_the_object_is_tolerated() {
    assert!(parse_theme_response(&format!("```json\n{LABELS_OK}\n```"), 2).is_some());
}

#[test]
fn a_partial_label_set_is_rejected_rather_than_filled_in() {
    // Defaulting the rest to 其他 would look like a classification the model
    // never made.
    assert!(parse_theme_response(LABELS_OK, 3).is_none());
}

#[test]
fn an_invented_theme_rejects_the_whole_response() {
    assert!(parse_theme_response(
        r#"{"labels":[{"id":1,"theme":"不存在"},{"id":2,"theme":"政策監管"}]}"#,
        2
    )
    .is_none());
}

#[test]
fn a_repeated_or_out_of_range_id_rejects_the_response() {
    assert!(parse_theme_response(
        r#"{"labels":[{"id":1,"theme":"產品發布"},{"id":1,"theme":"政策監管"}]}"#,
        2
    )
    .is_none());
    assert!(parse_theme_response(r#"{"labels":[{"id":0,"theme":"產品發布"}]}"#, 2).is_none());
}

#[test]
fn a_boolean_id_is_not_an_integer() {
    assert!(parse_theme_response(
        r#"{"labels":[{"id":true,"theme":"產品發布"},{"id":2,"theme":"政策監管"}]}"#,
        2
    )
    .is_none());
}

// ── theme layout ─────────────────────────────────────────────────────────────

fn five() -> Vec<String> {
    lines(&["- 甲", "- 乙", "- 丙", "- 丁", "- 戊"])
}

fn labels(pairs: &[(usize, &str)]) -> BTreeMap<usize, String> {
    pairs.iter().map(|(k, v)| (*k, v.to_string())).collect()
}

#[test]
fn a_theme_needs_two_stories_to_earn_a_heading() {
    let blocks = parse_ai_blocks(&five()).unwrap();
    let plan = theme_layout_plan(
        &blocks,
        &labels(&[
            (1, "產品發布"),
            (2, "產品發布"),
            (3, "政策監管"),
            (4, "政策監管"),
            (5, "其他"),
        ]),
    );
    assert_eq!(plan.headed, vec!["產品發布", "政策監管"]);
    assert_eq!(plan.tail, vec![4]); // the lone 其他, by block index
}

#[test]
fn all_singletons_means_no_headings_at_all() {
    let blocks = parse_ai_blocks(&five()).unwrap();
    let plan = theme_layout_plan(
        &blocks,
        &labels(&[
            (1, "產品發布"),
            (2, "研究突破"),
            (3, "政策監管"),
            (4, "產業資本"),
            (5, "其他"),
        ]),
    );
    assert!(plan.headed.is_empty());
    assert_eq!(plan.tail, vec![0, 1, 2, 3, 4]);
    // Nothing clusters, so rendering adds nothing and returns the flat lines.
    assert!(theme_render(&five(), &blocks, &labels(&[
        (1, "產品發布"), (2, "研究突破"), (3, "政策監管"),
        (4, "產業資本"), (5, "其他"),
    ]))
    .is_none());
}

#[test]
fn the_other_bucket_is_rendered_last() {
    let blocks = parse_ai_blocks(&five()).unwrap();
    let l = labels(&[
        (1, "其他"),
        (2, "其他"),
        (3, "政策監管"),
        (4, "政策監管"),
        (5, "政策監管"),
    ]);
    let out = theme_render(&five(), &blocks, &l).unwrap();
    assert_eq!(out[0], "▸ 政策監管");
    assert_eq!(out[4], "▸ 其他");
}

#[test]
fn a_paywall_pair_moves_under_its_heading_as_a_unit() {
    let src = lines(&[
        &format!("- 免費替代 [🔗]({L})"),
        &format!("{PAYWALL_CONT_PREFIX}原文：付費 [🔗]({L})  {PAYWALL_NOTE}"),
        "- 乙",
        "- 丙",
    ]);
    let blocks = parse_ai_blocks(&src).unwrap();
    let out = theme_render(
        &src,
        &blocks,
        &labels(&[(1, "產品發布"), (2, "產品發布"), (3, "政策監管")]),
    )
    .unwrap();
    let head = out.iter().position(|l| l.contains("免費替代")).unwrap();
    assert!(out[head + 1].starts_with(PAYWALL_CONT_PREFIX), "{out:?}");
}

#[test]
fn blank_separated_lines_are_never_regrouped() {
    // The block slices do not cover every line, so regrouping would delete the
    // blanks and possibly more. Fail flat instead.
    let src = lines(&["- 甲", "", "- 乙", "- 丙"]);
    let blocks = parse_ai_blocks(&src).unwrap();
    assert!(theme_render(
        &src,
        &blocks,
        &labels(&[(1, "產品發布"), (2, "產品發布"), (3, "政策監管")])
    )
    .is_none());
}

// ── cross-dedup ──────────────────────────────────────────────────────────────

#[test]
fn a_well_formed_grouping_parses() {
    let g = parse_cross_dedup_response(r#"{"groups":[{"members":[1,2],"keep":1}]}"#, 3).unwrap();
    assert_eq!(g, vec![Group { members: vec![1, 2], keep: 1 }]);
}

#[test]
fn no_duplicates_found_is_a_valid_answer() {
    assert_eq!(
        parse_cross_dedup_response(r#"{"groups":[]}"#, 3),
        Some(Vec::new())
    );
}

#[test]
fn a_malformed_group_rejects_every_group() {
    // Partial acceptance would let one bad group reshape the section.
    for bad in [
        r#"{"groups":[{"members":[1],"keep":1}]}"#,          // fewer than two
        r#"{"groups":[{"members":[1,1],"keep":1}]}"#,        // repeated member
        r#"{"groups":[{"members":[1,9],"keep":1}]}"#,        // out of range
        r#"{"groups":[{"members":[1,2],"keep":3}]}"#,        // keep is not a member
        r#"{"groups":[{"members":[1,2],"keep":1},{"members":[2,3],"keep":2}]}"#, // overlap
        r#"{"groups":"x"}"#,
        "no json",
    ] {
        assert!(
            parse_cross_dedup_response(bad, 3).is_none(),
            "accepted: {bad}"
        );
    }
}

#[test]
fn a_pair_is_counted_once_per_sample_regardless_of_member_order() {
    let samples = vec![
        vec![Group { members: vec![2, 1], keep: 1 }],
        vec![Group { members: vec![1, 2], keep: 1 }],
    ];
    assert_eq!(pair_votes(&samples), [((1, 2), 2)].into_iter().collect());
}

#[test]
fn a_group_of_three_contributes_all_three_pairs() {
    let samples = vec![vec![Group { members: vec![1, 2, 3], keep: 1 }]];
    let votes = pair_votes(&samples);
    assert_eq!(votes.len(), 3);
    assert_eq!(votes[&(1, 3)], 1);
}

#[test]
fn components_chain_only_through_pairs_that_survived_the_vote() {
    assert_eq!(components(&[(1, 2), (2, 3)], 4), vec![vec![1, 2, 3]]);
    assert_eq!(components(&[(1, 2)], 4), vec![vec![1, 2]]);
    assert!(components(&[], 4).is_empty());
}

#[test]
fn the_survivor_is_an_accessible_block_before_a_paywalled_one() {
    let src = lines(&[
        &format!("- 甲 {PAYWALL_NOTE}"),
        "- 乙",
        &format!("- 丙 {PAYWALL_NOTE}"),
    ]);
    let blocks = parse_ai_blocks(&src).unwrap();
    assert_eq!(survivor(&[1, 2], &blocks), 2);
    assert_eq!(survivor(&[3, 1], &blocks), 1); // both paywalled: lowest index
}

#[test]
fn a_normal_drop_is_applied() {
    let ten: Vec<String> = (1..=10).map(|i| format!("- 第{i}則")).collect();
    let out = apply_cross_dedup(&ten, &parse_ai_blocks(&ten).unwrap(), &[Group {
        members: vec![1, 2],
        keep: 1,
    }])
    .unwrap();
    assert_eq!(out.len(), 9);
    assert!(!out.contains(&"- 第2則".to_string()));
}

#[test]
fn a_drop_past_forty_percent_of_the_section_is_refused_wholesale() {
    // The ensemble cannot police itself — a correlated run swings aggressive
    // together — so the cap is the whole circuit breaker.
    let ten: Vec<String> = (1..=10).map(|i| format!("- 第{i}則")).collect();
    let blocks = parse_ai_blocks(&ten).unwrap();
    let drops = |members: Vec<usize>| {
        apply_cross_dedup(&ten, &blocks, &[Group { members, keep: 1 }])
    };
    // Four of ten is exactly the cap and is allowed; five is not.
    assert!(drops((1..=5).collect()).is_some(), "4 drops must pass");
    assert!(drops((1..=6).collect()).is_none(), "5 drops must be refused");
    assert!(drops((1..=10).collect()).is_none());
}

#[test]
fn a_two_block_section_may_still_lose_one() {
    // The ratio alone would floor at zero drops below four blocks, which would
    // make the pass useless on exactly the short sections it is aimed at.
    let two = lines(&["- 甲", "- 乙"]);
    let out = apply_cross_dedup(&two, &parse_ai_blocks(&two).unwrap(), &[Group {
        members: vec![1, 2],
        keep: 2,
    }])
    .unwrap();
    assert_eq!(out, vec!["- 乙"]);
}
