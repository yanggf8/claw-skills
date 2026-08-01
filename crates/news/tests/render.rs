//! Link attachment, trimming, and the Markdown probe.
//!
//! Cases are taken from a differential run against the Python, so each one
//! pins behaviour the live skill already has rather than whatever this port
//! happens to produce.

use news::render::*;
use std::collections::HashMap;

const LINK: &str = "https://example.com/a?b=1";

fn numbered() -> HashMap<u32, Numbered> {
    [
        (1, "甲標題", "https://n/1", "Reuters"),
        (2, "乙標題", "", ""),
        (3, "丙標題", "https://n/3", "AP"),
    ]
    .into_iter()
    .map(|(n, t, l, s)| {
        (
            n,
            Numbered {
                title: t.into(),
                link: l.into(),
                source_name: s.into(),
            },
        )
    })
    .collect()
}

fn no_paywall() -> HashMap<u32, Replacement> {
    HashMap::new()
}

// ── numbered link attachment ─────────────────────────────────────────────────

#[test]
fn a_marked_bullet_gets_its_source_link() {
    let (text, n) = attach_numbered_links("- #1 甲", &numbered(), &no_paywall());
    assert_eq!(text, "- #1 甲".replace("#1 ", "") + " [🔗](https://n/1)");
    assert_eq!(n, 1);
}

#[test]
fn an_item_with_no_link_still_renders_as_a_bullet_but_counts_nothing() {
    // The count is what the caller uses to decide the section failed, so a
    // link-less item must not inflate it.
    let (text, n) = attach_numbered_links("- #2 乙", &numbered(), &no_paywall());
    assert_eq!(text, "- 乙");
    assert_eq!(n, 0);
}

#[test]
fn an_unknown_marker_keeps_the_text_and_drops_the_link() {
    let (text, n) = attach_numbered_links("- #9 未知編號", &numbered(), &no_paywall());
    assert_eq!(text, "- 未知編號");
    assert_eq!(n, 0);
}

#[test]
fn markdown_specials_in_the_body_are_neutralised_on_the_way_out() {
    let (text, _) = attach_numbered_links("- #1 星號*標題", &numbered(), &no_paywall());
    assert!(text.contains("星號＊標題"), "{text}");
}

#[test]
fn scaffolding_survives_but_stray_prose_is_dropped() {
    let (text, _) = attach_numbered_links(
        "**區段**\n- #1 甲\n---\n垃圾行\n[hint_picked:none]",
        &numbered(),
        &no_paywall(),
    );
    assert_eq!(text, "**區段**\n- 甲 [🔗](https://n/1)\n---");
}

#[test]
fn a_paywalled_pick_with_a_replacement_renders_as_a_pair() {
    let paywall: HashMap<u32, Replacement> = [(
        1,
        Replacement {
            title_zh: "免費替代標題".into(),
            link: "https://free/1".into(),
        },
    )]
    .into();
    let (text, n) = attach_numbered_links("- #1 甲", &numbered(), &paywall);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "- 免費替代標題 [🔗](https://free/1)");
    assert_eq!(
        lines[1],
        format!("{PAYWALL_CONT_PREFIX}原文：甲 [🔗](https://n/1)  {PAYWALL_NOTE}")
    );
    // Two links, counted once each.
    assert_eq!(n, 2);
}

#[test]
fn a_paywalled_pick_with_no_replacement_degrades_to_one_bullet_and_a_note() {
    let paywall: HashMap<u32, Replacement> = [(3, Replacement::default())].into();
    let (text, n) = attach_numbered_links("- #3 丙", &numbered(), &paywall);
    assert_eq!(text, format!("- 丙 [🔗](https://n/3)  {PAYWALL_NOTE}"));
    assert_eq!(n, 1);
}

// ── fuzzy attachment ─────────────────────────────────────────────────────────

fn link_map() -> LinkMap {
    let mut m = LinkMap::default();
    m.insert("台積電法說".into(), LINK.into());
    m.insert("輝達財報優於預期".into(), "https://n/2".into());
    m
}

#[test]
fn an_exact_title_gets_its_link() {
    assert_eq!(
        attach_links("- 台積電法說", &link_map()),
        format!("- 台積電法說 [🔗]({LINK})")
    );
}

#[test]
fn a_reworded_title_still_reaches_its_source_by_substring() {
    assert_eq!(
        attach_links("- 台積電法說會", &link_map()),
        format!("- 台積電法說會 [🔗]({LINK})")
    );
}

#[test]
fn a_line_that_already_has_a_link_is_left_alone() {
    assert_eq!(attach_links("- 已有連結 [🔗](x)", &link_map()), "- 已有連結 [🔗](x)");
}

#[test]
fn a_non_bullet_line_is_left_alone() {
    assert_eq!(attach_links("不是條列", &link_map()), "不是條列");
}

// ── trimming ─────────────────────────────────────────────────────────────────

#[test]
fn stripping_a_link_keeps_the_bullet_readable() {
    assert_eq!(strip_links_keep_spacing(&format!("- 甲 [🔗]({LINK})")), "- 甲");
    assert_eq!(strip_links_keep_spacing("-甲"), "- 甲");
}

#[test]
fn visible_text_is_the_label_without_the_url() {
    assert_eq!(markdown_visible_text("- 甲 [🔗](https://x)"), "- 甲 🔗");
}

#[test]
fn trimming_drops_links_from_the_bottom_up() {
    let text = format!("- 甲 [🔗]({LINK})\n- 乙 [🔗]({LINK})");
    // 69 characters; 45 fits once the lower link goes.
    assert_eq!(
        trim_links_to_limit(&text, 45),
        format!("- 甲 [🔗]({LINK})\n- 乙")
    );
}

#[test]
fn a_limit_no_link_removal_can_reach_strips_them_all() {
    let text = format!("- 甲 [🔗]({LINK})\n- 乙 [🔗]({LINK})");
    assert_eq!(trim_links_to_limit(&text, 30), "- 甲\n- 乙");
}

#[test]
fn trimming_lines_takes_a_paywall_continuation_with_its_parent() {
    // Otherwise the digest ends on an orphaned 原文 note with no headline.
    let pair = format!(
        "- 免費替代 [🔗]({LINK})\n{PAYWALL_CONT_PREFIX}原文：付費標題 [🔗]({LINK})  {PAYWALL_NOTE}"
    );
    let trimmed = trim_lines_to_limit(&format!("{pair}\n- 尾巴"), 30);
    assert!(
        !trimmed.contains(PAYWALL_CONT_PREFIX),
        "orphaned continuation left behind: {trimmed}"
    );
}

#[test]
fn a_text_that_cannot_be_trimmed_by_lines_is_cut_with_a_marker() {
    let text = "沒有任何條列的一大段中文文字".repeat(20);
    let trimmed = trim_lines_to_limit(&text, 100);
    assert!(trimmed.ends_with("…（已截短）"), "{trimmed}");
    assert_eq!(trimmed.chars().count(), 100 - 20 + 7);
}

#[test]
fn a_limit_too_small_for_the_marker_just_truncates() {
    assert_eq!(trim_lines_to_limit("甲乙丙丁", 2), "甲乙");
}

// ── the Markdown probe ───────────────────────────────────────────────────────

#[test]
fn balanced_emphasis_is_safe() {
    assert!(markdown_chunk_is_safe("*bold* a_b_c `code`").is_ok());
}

#[test]
fn an_unmatched_asterisk_is_rejected() {
    assert_eq!(
        markdown_chunk_is_safe("*unmatched"),
        Err("unmatched asterisk")
    );
}

#[test]
fn emphasis_characters_inside_a_link_url_do_not_count() {
    // Counting them would push every ordinary digest into plaintext, which
    // expands each [🔗] into a bare URL.
    assert!(markdown_chunk_is_safe("[🔗](http://x/a_b) 甲乙").is_ok());
}

#[test]
fn an_unclosed_link_is_rejected() {
    assert_eq!(markdown_chunk_is_safe("[unclosed"), Err("unclosed link bracket"));
    assert_eq!(markdown_chunk_is_safe("[a](b"), Err("unclosed link url"));
}

#[test]
fn a_bracket_that_is_not_a_link_is_still_probed() {
    // "[label] plain" has no URL part, so its text is ordinary content.
    assert!(markdown_chunk_is_safe("[label] plain").is_ok());
    assert_eq!(
        markdown_chunk_is_safe("[label] plain*"),
        Err("unmatched asterisk")
    );
}

#[test]
fn a_trailing_backslash_is_rejected_but_an_escaped_one_is_not() {
    assert_eq!(
        markdown_chunk_is_safe("ends with backslash\\"),
        Err("trailing backslash")
    );
    assert!(markdown_chunk_is_safe("ends with double\\\\").is_ok());
}

#[test]
fn an_escaped_asterisk_does_not_count_toward_the_pair() {
    assert!(markdown_chunk_is_safe("escaped \\* star").is_ok());
}

// ── chunking ─────────────────────────────────────────────────────────────────

#[test]
fn a_short_body_is_a_single_chunk() {
    assert_eq!(split_message_preserving_lines("short", 50), vec!["short"]);
}

#[test]
fn chunks_are_measured_in_characters_not_bytes() {
    // 300 Chinese characters is 900 bytes. A byte-counted limit of 200 would
    // split this into five chunks instead of two.
    let body = "甲".repeat(300);
    assert_eq!(split_message_preserving_lines(&body, 200).len(), 2);
}

#[test]
fn a_paywall_pair_at_a_boundary_stays_together() {
    let head = "- 免費替代標題";
    let cont = format!("{PAYWALL_CONT_PREFIX}原文：付費標題  {PAYWALL_NOTE}");
    let filler = "- 填充";
    let per = filler.chars().count() + 1;
    let limit = 60;
    // Fill to just short of the limit, so the head is the line that would
    // normally trigger a flush and the continuation would open the next chunk.
    let n = (limit - head.chars().count() - 2) / per;
    let body = format!(
        "{}\n{head}\n{cont}\n{}",
        std::iter::repeat_n(filler, n).collect::<Vec<_>>().join("\n"),
        std::iter::repeat_n(filler, n).collect::<Vec<_>>().join("\n"),
    );
    for chunk in split_message_preserving_lines(&body, limit) {
        assert_eq!(
            chunk.contains("免費替代標題"),
            chunk.contains("原文：付費標題"),
            "pair separated in chunk:\n{chunk}"
        );
    }
}

#[test]
fn a_single_line_longer_than_the_limit_is_hard_split() {
    let chunks = split_message_preserving_lines(&"X".repeat(250), 100);
    assert_eq!(chunks.len(), 3);
    assert_eq!(chunks[0].chars().count(), 100);
    assert_eq!(chunks[2].chars().count(), 50);
}

#[test]
fn an_empty_body_produces_no_chunks_to_send() {
    assert_eq!(split_message_preserving_lines("", 50), vec![""]);
    assert!(split_message_preserving_lines(&"\n".repeat(200), 50)
        .iter()
        .all(|c| c.is_empty()));
}
