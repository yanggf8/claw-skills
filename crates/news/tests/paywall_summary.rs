//! Summarising a paywalled article that no free replacement covers.
//!
//! The case behind this is The Atlantic, 2026-08-12: `theatlantic.com` serves a
//! crawler the whole 2195-word piece and applies its wall to a browser, so the
//! body-marker check could never see it and only the host list stops it. No
//! free rewrite of that story existed in any language, so before this the
//! reader got a headline and nothing else.

use news::precheck::{body_is_summarisable, PaywallEntry, PaywallMap};
use news::render::{attach_numbered_links, Numbered, Replacement, PAYWALL_CONT_PREFIX, PAYWALL_NOTE};
use news::summarize::paywall_summary_lines;
use std::collections::HashMap;

// ── is there an article here at all ──────────────────────────────────────────

#[test]
fn a_full_article_is_summarisable_and_a_stub_is_not() {
    // The Atlantic piece measured 2195 words through the real fetch path; a
    // hard paywall answers with a nav-chrome stub an order of magnitude
    // smaller. Summarising the stub would produce a confident paragraph drawn
    // from menu labels.
    assert!(body_is_summarisable(2195));
    assert!(!body_is_summarisable(40));
    assert!(!body_is_summarisable(0));
}

// ── parsing the model's answer ───────────────────────────────────────────────

#[test]
fn two_or_three_chinese_lines_are_kept_in_order() {
    let out = paywall_summary_lines(
        "- 前沿模型在例行測試中突破內部系統連上公網\n\
         - OpenAI、Anthropic 與 Meta 均通報模型入侵了其他公司\n\
         - 逃逸的機器人曾嘗試發動社交工程攻擊",
    );
    assert_eq!(
        out.as_deref(),
        Some(
            "前沿模型在例行測試中突破內部系統連上公網\n\
             OpenAI、Anthropic 與 Meta 均通報模型入侵了其他公司\n\
             逃逸的機器人曾嘗試發動社交工程攻擊"
        )
    );
}

#[test]
fn a_refusal_yields_nothing() {
    assert_eq!(paywall_summary_lines("NO"), None);
    assert_eq!(paywall_summary_lines("- no\n"), None);
    assert_eq!(paywall_summary_lines(""), None);
    assert_eq!(paywall_summary_lines("   \n\n  "), None);
}

#[test]
fn an_english_line_is_dropped_while_its_chinese_neighbours_survive() {
    // The check is per line, not per block. A whole-block check passes this
    // input and puts an English sentence into a Chinese digest — after the
    // section language gate has already run, so nothing downstream catches it.
    let out = paywall_summary_lines(
        "- 前沿模型突破了內部系統\n\
         - Bots broke out of internal IT systems\n\
         - 業者事後才發現",
    );
    assert_eq!(out.as_deref(), Some("前沿模型突破了內部系統\n業者事後才發現"));
}

#[test]
fn no_more_than_three_lines_are_kept() {
    let out = paywall_summary_lines("- 第一句話\n- 第二句話\n- 第三句話\n- 第四句話\n- 第五句話")
        .expect("some lines");
    assert_eq!(out.lines().count(), 3);
    assert!(!out.contains("第四句話"), "{out}");
}

#[test]
fn a_markdown_link_in_a_summary_line_is_reduced_to_its_text() {
    assert_eq!(
        paywall_summary_lines("- [模型入侵了其他公司](https://example.com/a)").as_deref(),
        Some("模型入侵了其他公司")
    );
}

// ── how it renders ───────────────────────────────────────────────────────────

fn numbered() -> HashMap<u32, Numbered> {
    [(
        1,
        Numbered {
            title: "或許是該對 AI 感到恐慌的時候了".into(),
            link: "https://www.theatlantic.com/technology/2026/08/openai-hacks-panic/688264/".into(),
            source_name: "The Atlantic".into(),
        },
    )]
    .into()
}

#[test]
fn a_summary_hangs_under_the_paywalled_headline_as_continuation_lines() {
    let paywall: HashMap<u32, Replacement> = [(
        1,
        Replacement {
            summary: "前沿模型突破了內部系統\n業者事後才發現".into(),
            ..Default::default()
        },
    )]
    .into();
    let (text, links) = attach_numbered_links("- #1 或許是該對 AI 感到恐慌的時候了", &numbered(), &paywall);
    let lines: Vec<&str> = text.lines().collect();

    assert!(lines[0].contains(PAYWALL_NOTE), "{text}");
    // The continuation prefix is what makes the chunker keep these with their
    // headline; without it a summary can be split into the next message.
    assert_eq!(lines[1], format!("{PAYWALL_CONT_PREFIX}前沿模型突破了內部系統"));
    assert_eq!(lines[2], format!("{PAYWALL_CONT_PREFIX}業者事後才發現"));
    // Only the original's own link — a summary adds no second link.
    assert_eq!(links, 1);
}

#[test]
fn a_free_replacement_wins_over_a_summary() {
    // Both populated is not a state the pipeline produces, but the renderer
    // must still prefer the free article: a link the reader can open beats
    // this repo restating the paid one.
    let paywall: HashMap<u32, Replacement> = [(
        1,
        Replacement {
            title_zh: "免費替代標題".into(),
            link: "https://free/1".into(),
            summary: "不該出現的摘要".into(),
        },
    )]
    .into();
    let (text, _) = attach_numbered_links("- #1 原標題", &numbered(), &paywall);
    assert!(text.contains("免費替代標題"), "{text}");
    assert!(!text.contains("不該出現的摘要"), "{text}");
}

#[test]
fn no_summary_and_no_replacement_still_degrades_to_one_bullet_and_a_note() {
    let paywall: HashMap<u32, Replacement> = [(1, Replacement::default())].into();
    let (text, _) = attach_numbered_links("- #1 原標題", &numbered(), &paywall);
    assert_eq!(text.lines().count(), 1, "{text}");
    assert!(text.contains(PAYWALL_NOTE), "{text}");
}

#[test]
fn a_summary_is_only_carried_for_an_entry_with_no_replacement() {
    // `render_replacements` is the boundary: it must not hand the renderer a
    // summary that will never be shown, because every rendered line still
    // costs width and chunk budget.
    let mut m = PaywallMap::new();
    m.insert(
        1,
        PaywallEntry {
            title: "有替代".into(),
            replacement: Some(Replacement {
                title_zh: "免費替代".into(),
                link: "https://free/1".into(),
                ..Default::default()
            }),
            summary: Some("這則不該帶摘要".into()),
            ..Default::default()
        },
    );
    m.insert(
        2,
        PaywallEntry {
            title: "沒替代".into(),
            replacement: None,
            summary: Some("這則要帶摘要".into()),
            ..Default::default()
        },
    );

    let out = news::precheck::render_replacements(&m);
    assert_eq!(out[&1].summary, "", "a replaced entry must carry no summary");
    assert_eq!(out[&2].summary, "這則要帶摘要");
}
