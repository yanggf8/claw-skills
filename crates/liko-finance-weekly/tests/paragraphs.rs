//! The issue body as dev.to renders it.
//!
//! Every issue up to 2026-11-22 rendered as one `<p>`: the body is plain lines,
//! and Markdown joins consecutive lines. These tests render with a CommonMark
//! parser and count paragraphs, so they fail on the defect itself rather than
//! on a particular way of writing the fix.

use liko_finance_weekly::proc::paragraphs;
use pulldown_cmark::{Event, Parser, Tag};

/// The 2026-11-22 issue's shape: three plain heading lines, prose lines under
/// the first two, three action lines under the last. Text shortened.
const BODY: &str = "本週訊號
本週最值得檢視的，是家族傳承架構正從單次移轉走向長期治理。
同一週的壽險數據也值得放在傳承角色下看。
對應檢視面向
第一個面向是傳承。
第二個面向是流動性與保單角色。
本週檢視動作
確認現有信託、保單受益安排與遺囑是否一致。
盤點未來三年家庭可動用的現金流。
詢問稅務師連續受益人信託的適用條件。";

/// The text of each top-level paragraph, in order. Soft and hard breaks inside
/// a paragraph are kept as `\n`, so a paragraph that swallowed several lines
/// shows it.
fn rendered_paragraphs(markdown: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current: Option<String> = None;
    for event in Parser::new(markdown) {
        match event {
            Event::Start(Tag::Paragraph) => current = Some(String::new()),
            Event::End(Tag::Paragraph) => out.extend(current.take()),
            Event::Text(t) => {
                if let Some(p) = current.as_mut() {
                    p.push_str(&t);
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if let Some(p) = current.as_mut() {
                    p.push('\n');
                }
            }
            _ => {}
        }
    }
    out
}

#[test]
fn the_plain_body_renders_as_one_paragraph() {
    // The defect, reproduced: without this the tests below could pass against
    // a parser that never merged lines in the first place.
    assert_eq!(rendered_paragraphs(BODY).len(), 1);
}

#[test]
fn every_line_becomes_its_own_paragraph() {
    let expected: Vec<String> = BODY.lines().map(str::to_string).collect();
    assert_eq!(rendered_paragraphs(&paragraphs(BODY)), expected);
}

#[test]
fn each_section_heading_stands_alone() {
    let rendered = rendered_paragraphs(&paragraphs(BODY));
    for heading in ["本週訊號", "對應檢視面向", "本週檢視動作"] {
        assert!(
            rendered.iter().any(|p| p == heading),
            "{heading} is not a paragraph of its own: {rendered:?}"
        );
    }
}

#[test]
fn blank_lines_the_model_already_wrote_are_not_doubled() {
    let spaced = "本週訊號\n\n\n內容一\n   \n對應檢視面向\n內容二";
    assert_eq!(paragraphs(spaced), "本週訊號\n\n內容一\n\n對應檢視面向\n\n內容二");
}

#[test]
fn an_indented_line_does_not_become_a_code_block() {
    // Four leading spaces after a blank line is an indented code block in
    // CommonMark; the line must still render as a paragraph.
    let indented = "本週檢視動作\n    確認受益人安排。";
    assert_eq!(
        rendered_paragraphs(&paragraphs(indented)),
        vec!["本週檢視動作".to_string(), "確認受益人安排。".to_string()]
    );
}

#[test]
fn applying_it_twice_changes_nothing() {
    // The repair pass rewrites an already-shaped body, and the result is
    // shaped again.
    let once = paragraphs(BODY);
    assert_eq!(paragraphs(&once), once);
}
