//! Numbering, pair hints, and the post-selection collapse.

use news::render::Numbered;
use news::select::*;
use news::text::Item;

fn n(entries: &[(u32, &str)]) -> NumberedMap {
    entries
        .iter()
        .map(|(i, t)| {
            (
                *i,
                Numbered {
                    title: (*t).into(),
                    link: "https://n/1".into(),
                    source_name: String::new(),
                },
            )
        })
        .collect()
}

/// Two rewrites of one earnings story, one unrelated story, and a pair of
/// English rewrites — the shapes the collapse exists for.
fn corpus() -> NumberedMap {
    n(&[
        (1, "台積電第三季財報優於預期 毛利率創新高"),
        (2, "台積電第三季財報 毛利率創新高 法人上調目標價"),
        (3, "輝達推出新一代 GPU 架構"),
        (4, "OpenAI announces GPT next generation model release"),
        (5, "OpenAI announces GPT next generation model launch"),
    ])
}

// ── numbering ────────────────────────────────────────────────────────────────

#[test]
fn items_are_numbered_across_sections_in_one_sequence() {
    let items = vec![
        (
            "ai".to_string(),
            vec![
                Item { title: "甲 - Reuters".into(), link: "https://a/1".into(), ..Default::default() },
                Item { title: "乙 - AP".into(), link: "https://a/2".into(), ..Default::default() },
            ],
        ),
        (
            "tech".to_string(),
            vec![Item { title: "丙".into(), ..Default::default() }],
        ),
    ];
    let (numbered, raw) = number_items_for_prompt(&items, None, &|_| None);
    assert_eq!(numbered.len(), 3);
    assert_eq!(numbered[&3].title, "丙");
    assert_eq!(raw, "[ai]\n  #1 甲 - Reuters\n  #2 乙 - AP\n[tech]\n  #3 丙");
}

#[test]
fn an_empty_section_contributes_no_heading() {
    let items = vec![
        ("ai".to_string(), vec![Item { title: "甲".into(), ..Default::default() }]),
        ("empty".to_string(), vec![]),
    ];
    let (_, raw) = number_items_for_prompt(&items, None, &|_| None);
    assert!(!raw.contains("[empty]"), "{raw}");
}

#[test]
fn a_zero_limit_drops_the_section_entirely() {
    let items = vec![("ai".to_string(), vec![Item { title: "甲".into(), ..Default::default() }])];
    let (numbered, raw) = number_items_for_prompt(&items, None, &|_| Some(0));
    assert!(numbered.is_empty());
    assert_eq!(raw, "[ai]\n");
}

// ── pair hints ───────────────────────────────────────────────────────────────

#[test]
fn only_independent_pairs_are_reported_never_a_closure() {
    // A shares tokens with B and B with C; that is two hints, not one group of
    // three. Transitive closure here would tell the model to merge C into A.
    let pairs = dedup_pair_hints(&corpus(), 4);
    assert!(pairs.iter().all(|(a, b, _)| a < b));
    assert!(pairs.contains(&(4, 5, pairs.iter().find(|p| p.0 == 4).unwrap().2)));
}

#[test]
fn a_higher_threshold_reports_fewer_pairs() {
    // The two Chinese rewrites share 12 tokens, the two English ones 6.
    assert_eq!(dedup_pair_hints(&corpus(), 6).len(), 2);
    assert_eq!(dedup_pair_hints(&corpus(), 8), vec![(1, 2, 12)]);
}

#[test]
fn a_title_with_no_significant_tokens_pairs_with_nothing() {
    let m = n(&[(1, "短"), (2, ""), (3, "短")]);
    assert!(dedup_pair_hints(&m, 1).is_empty());
}

#[test]
fn the_hint_block_is_empty_when_there_is_nothing_to_hint() {
    assert_eq!(format_dedup_hint_block(&[]), "");
}

#[test]
fn the_hint_block_lists_every_pair_on_one_line() {
    assert_eq!(
        format_dedup_hint_block(&[(1, 2, 4), (4, 5, 6)]),
        "可能同事件候選（僅供複核，仍按事件語義判斷；非硬性合併）：#1+#2; #4+#5\n\n"
    );
}

// ── pick range ───────────────────────────────────────────────────────────────

#[test]
fn a_pick_range_yields_its_lower_bound() {
    assert_eq!(parse_pick_min(Some("3-5")), Some(3));
    assert_eq!(parse_pick_min(Some(" 7 ")), Some(7));
    assert_eq!(parse_pick_min(Some("2")), Some(2));
}

#[test]
fn a_pick_spec_that_does_not_start_with_a_number_has_no_floor() {
    assert_eq!(parse_pick_min(Some("abc")), None);
    assert_eq!(parse_pick_min(Some("-5")), None);
    assert_eq!(parse_pick_min(None), None);
}

// ── the collapse ─────────────────────────────────────────────────────────────

fn collapse(summary: &str, pick_min: Option<u32>) -> String {
    post_dedup_selected_summary(summary, &corpus(), "test", 4, pick_min)
}

#[test]
fn two_rewrites_of_one_event_collapse_to_the_first() {
    let out = collapse("- #1 甲\n- #2 乙\n- #3 丙", None);
    assert_eq!(out, "- #1 甲\n- #3 丙");
}

#[test]
fn a_single_selection_is_left_alone() {
    assert_eq!(collapse("- #1 甲", None), "- #1 甲");
}

#[test]
fn scaffolding_around_the_bullets_survives_the_collapse() {
    let out = collapse("**區段**\n- #1 甲\n- #2 乙\n- #3 丙\n---", None);
    assert_eq!(out, "**區段**\n- #1 甲\n- #3 丙\n---");
}

#[test]
fn a_weak_bridge_cannot_transitively_delete_an_unrelated_story() {
    // The collapse is greedy against the already-kept set, not connected
    // components: an A-B and a B-C edge must not remove C when A and C barely
    // overlap.
    let m = n(&[
        (1, "台積電 法說會 財報 毛利率 展望"),
        (2, "台積電 法說會 財報 毛利率 輝達 GPU 架構 發表"),
        (3, "輝達 GPU 架構 發表 效能"),
    ]);
    let out = post_dedup_selected_summary("- #1 a\n- #2 b\n- #3 c", &m, "t", 4, None);
    // #2 collides with #1 and goes; #3 is compared against #1 only, and stays.
    assert_eq!(out, "- #1 a\n- #3 c");
}

#[test]
fn an_underfilled_section_is_topped_up_from_candidates_the_model_skipped() {
    // The model picked three; #1 and #2 are the same event so the collapse
    // leaves two, under a floor of three. #3 was never selected and is
    // unrelated, so it fills the gap — carrying its own title, since there is
    // no model-written bullet for it.
    let out = collapse("- #1 甲\n- #2 乙\n- #4 d", Some(3));
    assert_eq!(out, "- #1 甲\n- #4 d\n- #3 輝達推出新一代 GPU 架構");
}

#[test]
fn a_shortfall_the_model_itself_created_is_not_topped_up() {
    // Two picked, one survives, floor of three: the collapse did not cause the
    // shortfall relative to what the model chose, so nothing is added.
    assert_eq!(collapse("- #1 甲\n- #2 乙", Some(3)), "- #1 甲");
}

#[test]
fn the_refill_never_revives_a_bullet_the_collapse_just_dropped() {
    // #4 and #5 are the same English story. With a floor of five, the refill
    // may add #1/#2/#3 but must never bring #5 back — it is a duplicate by
    // construction.
    assert_eq!(collapse("- #4 d\n- #5 e", Some(5)), "- #4 d");
}

#[test]
fn nothing_is_topped_up_when_the_model_underfilled_on_its_own() {
    // The floor only applies to *collapse*-driven shortfall. A model that
    // simply picked one item was answering the prompt, and inventing extra
    // bullets for it would override its judgement.
    let out = collapse("- #1 甲", Some(3));
    assert_eq!(out, "- #1 甲");
}
