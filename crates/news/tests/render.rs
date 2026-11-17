//! Link attachment, trimming, and the Markdown probe.
//!
//! Cases are taken from a differential run against the Python, so each one
//! pins behaviour the live skill already has rather than whatever this port
//! happens to produce.

use news::render::*;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;

const LINK: &str = "https://example.com/a?b=1";

/// A scratch `HOME`, set once for the whole binary.
///
/// `attach_numbered_links` writes trace events to `$HOME/.nullclaw/`, so
/// without this every run of this suite appended fixture events to the
/// operator's real trace file. Set through a `OnceLock` that every renderer
/// test passes before it can reach a trace write, so no thread reads `HOME`
/// while another is setting it.
fn scratch_home() -> &'static PathBuf {
    static HOME: OnceLock<PathBuf> = OnceLock::new();
    HOME.get_or_init(|| {
        let home = std::env::temp_dir().join(format!("news-render-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join(".nullclaw")).unwrap();
        std::env::set_var("HOME", &home);
        home
    })
}

fn numbered() -> HashMap<u32, Numbered> {
    scratch_home();
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
            ..Default::default()
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

// ── degenerate model-written headlines ───────────────────────────────────────
//
// 2026-11-13: the 重大新聞 section shipped `- 川普喊 [🔗](…)`. The model wrote
// a three-character fragment for a full headline and nothing between it and the
// reader looked at the body.

/// Google News titles, suffix included, as the renderer receives them.
fn headline_sources() -> HashMap<u32, Numbered> {
    scratch_home();
    [
        // 14 CJK before the suffix.
        (5, "川普喊話伊朗限期談判否則開戰 - 中央社", "https://n/5"),
        (6, "Trump says Iran must negotiate within two weeks or face war - Reuters", "https://n/6"),
        // Exactly 10 CJK: the floor, inclusive. 戰 makes it provably
        // Traditional, so only the floor decides.
        (7, "川普喊話伊朗限期開戰 - 中央社", "https://n/7"),
        // Exactly 9 CJK: one under the floor, and provably Traditional.
        (8, "川普喊話伊朗限期戰 - 中央社", "https://n/8"),
        // Markdown specials in the source itself.
        (9, "長科*成關鍵受惠股帶動半導體類股上漲 - 經濟日報", "https://n/9"),
        // Japanese: 11 ideographs, so it clears the CJK floor, but kana mark
        // it as not Chinese.
        (10, "OpenAIが新型AIモデルを発表、企業向け機能を強化 - 日経", "https://n/10"),
        // Simplified Chinese: 15 ideographs, carrying 发 and 业.
        (11, "OpenAI发布新一代模型 企业功能全面升级 - 新浪科技", "https://n/11"),
        // Traditional, with the katakana middle dot Taiwanese outlets use
        // between transliterated names.
        (12, "唐納・川普宣布對伊朗實施新一輪制裁 - 中央社", "https://n/12"),
        // Simplified, using none of the Simplified-only marker characters.
        (13, "人工智能图像识别安全研究 - 科技日報", "https://n/13"),
        // Japanese written entirely in kanji: no kana to give it away.
        (14, "AI半導体性能比較検証結果 - 日経", "https://n/14"),
        // Traditional, but with none of the provably-Traditional characters:
        // 話 談 開 則 are written the same in Japanese.
        (15, "川普喊話伊朗限期談判否則開打 - 中央社", "https://n/15"),
    ]
    .into_iter()
    .map(|(n, t, l)| {
        (
            n,
            Numbered {
                title: t.into(),
                link: l.into(),
                source_name: String::new(),
            },
        )
    })
    .collect()
}

#[test]
fn a_fragment_written_for_a_cjk_headline_is_replaced_by_the_source_title() {
    let (text, n) = attach_numbered_links("- #5 川普喊", &headline_sources(), &no_paywall());
    assert_eq!(text, "- 川普喊話伊朗限期談判否則開戰 [🔗](https://n/5)");
    assert_eq!(n, 1);
}

#[test]
fn a_substituted_source_title_is_markdown_neutralised() {
    let (text, _) = attach_numbered_links("- #9 長科", &headline_sources(), &no_paywall());
    assert_eq!(text, "- 長科＊成關鍵受惠股帶動半導體類股上漲 [🔗](https://n/9)");
}

#[test]
fn a_short_headline_for_an_english_source_is_left_alone() {
    // The section is Chinese; injecting the English original would be worse
    // than a terse translation.
    let (text, n) = attach_numbered_links("- #6 川普喊", &headline_sources(), &no_paywall());
    assert_eq!(text, "- 川普喊 [🔗](https://n/6)");
    assert_eq!(n, 1);
}

#[test]
fn a_condensed_headline_at_the_floor_is_kept() {
    // Six CJK is a legitimate condensation of a fourteen-character title.
    let (text, _) = attach_numbered_links("- #5 川普喊話伊朗", &headline_sources(), &no_paywall());
    assert_eq!(text, "- 川普喊話伊朗 [🔗](https://n/5)");
}

#[test]
fn one_cjk_under_the_body_floor_is_replaced() {
    let (text, _) = attach_numbered_links("- #5 川普喊話伊", &headline_sources(), &no_paywall());
    assert_eq!(text, "- 川普喊話伊朗限期談判否則開戰 [🔗](https://n/5)");
}

#[test]
fn a_normal_rewritten_headline_is_kept() {
    let (text, _) =
        attach_numbered_links("- #5 川普限伊朗兩週內談判", &headline_sources(), &no_paywall());
    assert_eq!(text, "- 川普限伊朗兩週內談判 [🔗](https://n/5)");
}

#[test]
fn a_source_title_at_the_floor_is_substantial_enough_to_substitute() {
    let (text, _) = attach_numbered_links("- #7 川普喊", &headline_sources(), &no_paywall());
    assert_eq!(text, "- 川普喊話伊朗限期開戰 [🔗](https://n/7)");
}

#[test]
fn a_genuinely_short_source_title_does_not_count_as_degenerate() {
    // Nine CJK: a short body may simply be a faithful short headline.
    let (text, _) = attach_numbered_links("- #8 川普喊", &headline_sources(), &no_paywall());
    assert_eq!(text, "- 川普喊 [🔗](https://n/8)");
}

#[test]
fn the_paywall_original_line_carries_the_repaired_body() {
    let paywall: HashMap<u32, Replacement> = [(
        5,
        Replacement {
            title_zh: "川普要求伊朗兩週內回到談判桌".into(),
            link: "https://free/5".into(),
            ..Default::default()
        },
    )]
    .into();
    let (text, n) = attach_numbered_links("- #5 川普喊", &headline_sources(), &paywall);
    assert_eq!(
        text,
        format!(
            "- 川普要求伊朗兩週內回到談判桌 [🔗](https://free/5)\n\
             {PAYWALL_CONT_PREFIX}原文：川普喊話伊朗限期談判否則開戰 [🔗](https://n/5)  {PAYWALL_NOTE}"
        )
    );
    assert_eq!(n, 2);
}

#[test]
fn the_degraded_paywall_bullet_carries_the_repaired_body() {
    let paywall: HashMap<u32, Replacement> = [(5, Replacement::default())].into();
    let (text, _) = attach_numbered_links("- #5 川普喊", &headline_sources(), &paywall);
    assert_eq!(
        text,
        format!("- 川普喊話伊朗限期談判否則開戰 [🔗](https://n/5)  {PAYWALL_NOTE}")
    );
}

#[test]
fn a_short_headline_for_a_japanese_source_is_left_alone() {
    // Ideographs alone do not make a title Chinese; substituting this would
    // put Japanese into a Traditional Chinese digest.
    let (text, _) =
        attach_numbered_links("- #10 OpenAI 新模型", &headline_sources(), &no_paywall());
    assert_eq!(text, "- OpenAI 新模型 [🔗](https://n/10)");
}

#[test]
fn a_short_headline_for_a_simplified_chinese_source_is_left_alone() {
    let (text, _) =
        attach_numbered_links("- #11 OpenAI 新模型", &headline_sources(), &no_paywall());
    assert_eq!(text, "- OpenAI 新模型 [🔗](https://n/11)");
}

#[test]
fn a_simplified_source_without_a_marker_character_is_left_alone() {
    let (text, _) = attach_numbered_links("- #13 AI 新研究", &headline_sources(), &no_paywall());
    assert_eq!(text, "- AI 新研究 [🔗](https://n/13)");
}

#[test]
fn an_all_kanji_japanese_source_is_left_alone() {
    let (text, _) = attach_numbered_links("- #14 AI 新研究", &headline_sources(), &no_paywall());
    assert_eq!(text, "- AI 新研究 [🔗](https://n/14)");
}

#[test]
fn a_traditional_source_that_cannot_be_proven_traditional_is_left_alone() {
    // The accepted cost of failing closed: this is Traditional Chinese, but
    // every character in it is shared with Japanese or Simplified, so the
    // fragment ships as written and is only traced.
    let (text, _) = attach_numbered_links("- #15 川普喊", &headline_sources(), &no_paywall());
    assert_eq!(text, "- 川普喊 [🔗](https://n/15)");
}

#[test]
fn a_name_separator_dot_does_not_make_a_traditional_title_japanese() {
    let (text, _) = attach_numbered_links("- #12 川普制裁", &headline_sources(), &no_paywall());
    assert_eq!(text, "- 唐納・川普宣布對伊朗實施新一輪制裁 [🔗](https://n/12)");
}

#[test]
fn a_repair_is_recorded_in_the_trace() {
    // A unique fragment, so parallel tests writing the same file cannot match.
    let (_, _) = attach_numbered_links(
        "- #5 川普又喊\n- #8 川普再喊\n- #6 川普還喊\n- #10 日經又報\n- #11 新浪又報\n- #15 川普又打",
        &headline_sources(),
        &no_paywall(),
    );
    let log = std::fs::read_to_string(scratch_home().join(".nullclaw/skill-traces.jsonl"))
        .expect("trace file written");
    let event: serde_json::Value = log
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .find(|v| {
            v["event"] == "headline_degenerate"
                && v["items"].as_array().is_some_and(|a| a.iter().any(|i| i["written"] == "川普又喊"))
        })
        .expect("headline_degenerate event for this call");
    // #8 (short source) and #6 (English source) are not degenerate. #10, #11
    // and #15 are, but their sources cannot be shown to be Traditional
    // Chinese, so they are reported and left as written.
    assert_eq!(event["count"], 4);
    assert_eq!(
        event["items"],
        serde_json::json!([
            {
                "n": 5,
                "written": "川普又喊",
                "source": "川普喊話伊朗限期談判否則開戰",
                "replaced": true,
            },
            {
                "n": 10,
                "written": "日經又報",
                "source": "OpenAIが新型AIモデルを発表、企業向け機能を強化",
                "replaced": false,
            },
            {
                "n": 11,
                "written": "新浪又報",
                "source": "OpenAI发布新一代模型 企业功能全面升级",
                "replaced": false,
            },
            {
                "n": 15,
                "written": "川普又打",
                "source": "川普喊話伊朗限期談判否則開打",
                "replaced": false,
            },
        ])
    );
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
