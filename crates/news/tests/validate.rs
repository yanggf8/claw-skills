//! Tests for the reply gates.

use news::validate::*;
use std::collections::HashSet;

fn nums(v: &[u32]) -> HashSet<u32> { v.iter().copied().collect() }

// ── marker parsing ───────────────────────────────────────────────────────────

#[test]
fn a_leading_marker_is_read_with_or_without_a_bullet() {
    assert_eq!(leading_marker("#3 台積電法說"), Some(3));
    assert_eq!(leading_marker("- #3 台積電法說"), Some(3));
    assert_eq!(leading_marker("  -  #12 x"), Some(12));
}

#[test]
fn a_marker_followed_by_a_comma_is_prose_not_a_marker() {
    // A model writing "見 #3, #7 兩則" would otherwise have its prose counted
    // as a marked bullet.
    assert_eq!(leading_marker("#3, #7 兩則相關"), None);
}

#[test]
fn a_marker_running_into_a_word_is_not_a_marker() {
    assert_eq!(leading_marker("#3rd quarter results"), None);
}

#[test]
fn a_marker_elsewhere_in_the_line_does_not_count() {
    assert_eq!(leading_marker("台積電 #3 法說"), None);
}

#[test]
fn stripping_removes_the_marker_and_the_bullet() {
    assert_eq!(strip_marker_prefix("- #3  台積電法說"), "台積電法說");
    assert_eq!(strip_marker_prefix("台積電法說"), "台積電法說");
}

// ── bullet extraction ────────────────────────────────────────────────────────

#[test]
fn horizontal_rules_are_not_bullets() {
    assert!(news_bullet_lines("---\n--\n***").is_empty());
}

#[test]
fn the_no_news_placeholder_is_not_a_bullet() {
    // Nine call sites treat an empty result as "route to the placeholder".
    // Counting the placeholder itself would make that test never fire.
    assert!(news_bullet_lines("- 今日無相關新聞").is_empty());
}

#[test]
fn an_ellipsis_continuation_is_not_a_bullet() {
    assert!(news_bullet_lines("- ...續前").is_empty());
}

#[test]
fn reasoning_prose_is_invisible_to_the_bullet_list_but_visible_to_the_shape_gate() {
    // The two helpers mean different things and this is the case that shows it.
    // A reply that is only chain-of-thought must yield no bullets (so the
    // placeholder path runs) while still failing the shape gate (so it is never
    // delivered as a digest).
    let cot = "Let me think about which items matter.\nThe first is clearly the most important.";
    assert!(news_bullet_lines(cot).is_empty());
    assert_eq!(content_lines(cot).len(), 2);
    assert!(!shape_ok(cot, &nums(&[1, 2])));
}

#[test]
fn bracket_instrumentation_is_not_content() {
    // Stripped later during link attachment; failing the gate on it would
    // reject good replies.
    assert!(content_lines("[trace: x]").is_empty());
}

#[test]
fn a_bold_heading_is_framing_not_content() {
    assert!(content_lines("**產業動態**").is_empty());
}

// ── shape gate ───────────────────────────────────────────────────────────────

#[test]
fn every_content_line_must_carry_a_known_marker() {
    assert!(shape_ok("- #1 甲\n- #2 乙", &nums(&[1, 2])));
}

#[test]
fn a_marker_naming_an_item_we_never_offered_fails() {
    // The model invented an id, so its mapping back to a source is a guess.
    assert!(!shape_ok("- #1 甲\n- #9 乙", &nums(&[1, 2])));
}

#[test]
fn one_unmarked_line_fails_the_whole_reply() {
    assert!(!shape_ok("- #1 甲\n- 乙沒有標記", &nums(&[1, 2])));
}

#[test]
fn an_empty_reply_fails_rather_than_vacuously_passing() {
    // A model that returned nothing has not passed a shape check.
    assert!(!shape_ok("", &nums(&[1])));
    assert!(!shape_ok("\n\n---\n", &nums(&[1])));
}

// ── marker stats and order ───────────────────────────────────────────────────

#[test]
fn marker_stats_count_only_known_ids() {
    assert_eq!(marker_stats("- #1 甲\n- #9 乙", &nums(&[1, 2])), (1, 2));
}

#[test]
fn marker_ids_come_back_in_the_models_order_without_repeats() {
    assert_eq!(
        leading_marker_ids("- #3 丙\n- #1 甲\n- #3 丙又一次", &nums(&[1, 2, 3])),
        vec![3, 1]
    );
}

// ── language gate ────────────────────────────────────────────────────────────

#[test]
fn a_chinese_digest_passes() {
    assert!(language_ok("- #1 台積電法說會釋出樂觀展望\n- #2 輝達財報優於預期"));
}

#[test]
fn an_english_digest_fails() {
    assert!(!language_ok("- #1 TSMC guidance beats\n- #2 Nvidia earnings top forecasts"));
}

#[test]
fn a_bullet_turning_chinese_only_after_a_long_english_run_does_not_count_as_chinese() {
    // The first CJK character must appear within the opening 18. A line that
    // opens in English and switches halfway is the failure this measures.
    let (chinese, total) = language_stats("- #1 The company said on Thursday that 台積電 will expand");
    assert_eq!(total, 1);
    assert_eq!(chinese, 0);
}

#[test]
fn one_english_adverb_rejects_an_otherwise_chinese_digest() {
    // Half-translated prose reads worse than either language alone.
    assert!(!language_ok(
        "- #1 台積電法說會釋出樂觀展望\n- #2 輝達財報 significantly 優於預期\n- #3 記憶體股走弱\n- #4 政策收緊\n- #5 產能滿載"
    ));
}

#[test]
fn an_adverb_in_ascii_brackets_is_still_caught() {
    assert!(!language_ok("- #1 台積電展望樂觀 (however)\n- #2 輝達財報亮眼"));
}

#[test]
fn an_adverb_glued_to_full_width_punctuation_escapes_the_gate() {
    // Recording the boundary, not endorsing it. The check splits on whitespace
    // and trims ASCII punctuation only, so 樂觀（however） is one token that
    // matches nothing. Both implementations behave this way; a fix would be a
    // behaviour change, not a port.
    assert!(language_ok("- #1 台積電展望樂觀（however）\n- #2 輝達財報亮眼"));
}

// ── bracket instrumentation edges ────────────────────────────────────────────

#[test]
fn a_bracket_may_contain_an_opening_bracket_and_still_be_noise() {
    assert!(content_lines("[a[b]").is_empty());
}

#[test]
fn a_bracket_followed_by_text_is_content_and_must_face_the_gate() {
    // Closing early means the rest is prose the model wrote, not instrumentation.
    assert_eq!(content_lines("[a]b]"), vec!["[a]b]"]);
}

#[test]
fn a_bare_ellipsis_continuation_is_invisible_to_the_shape_gate() {
    // With or without a bullet — a truncation artefact is not a claim to check.
    assert!(content_lines("- ...續前").is_empty());
    assert!(content_lines("...裸露的續行").is_empty());
}

#[test]
fn a_proper_noun_that_merely_looks_english_is_fine() {
    assert!(language_ok("- #1 台積電與 Nvidia 簽約\n- #2 蘋果發表新機"));
}

#[test]
fn an_empty_digest_fails_the_language_gate_too() {
    assert!(!language_ok(""));
}

// ── markdown neutralisation ──────────────────────────────────────────────────

#[test]
fn asterisks_and_underscores_become_full_width() {
    // Telegram's legacy parser rejects the message otherwise, and Taiwanese
    // stock notation uses * literally.
    assert_eq!(neutralize_markdown("長科*成關鍵受惠股"), "長科＊成關鍵受惠股");
    assert_eq!(neutralize_markdown("a_b"), "a＿b");
}

#[test]
fn a_marker_id_too_large_for_u32_is_still_a_marker() {
    // Found by differential, not by reading the code: rejecting the parse made
    // the marker invisible, which left its digits in the bullet body and made
    // the language gate reject a perfectly good Chinese digest. Python has
    // bignums and no such ceiling.
    assert_eq!(leading_marker("- #999999999999999999999 溢位"), Some(u32::MAX));
    assert_eq!(strip_marker_prefix("- #999999999999999999999 溢位"), "溢位");
    assert!(language_ok("- #999999999999999999999 溢位"));
    // It still names no real item.
    assert!(!shape_ok("- #999999999999999999999 溢位", &nums(&[1, 2])));
}

#[test]
fn leading_zeros_name_the_same_item() {
    assert_eq!(leading_marker("- #0001 甲"), Some(1));
}
