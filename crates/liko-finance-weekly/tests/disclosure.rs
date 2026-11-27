//! The AI-assistance disclosure at the top of every issue.

use liko_finance_weekly::proc::{paragraphs, with_disclosure, AI_DISCLOSURE};

const BODY: &str = "本週訊號
本週最值得檢視的，是家族傳承架構正從單次移轉走向長期治理。
對應檢視面向
第一個面向是傳承。
本週檢視動作
確認現有信託、保單受益安排與遺囑是否一致。
盤點未來三年家庭可動用的現金流。";

#[test]
fn the_disclosure_is_the_first_paragraph() {
    let out = with_disclosure(&paragraphs(BODY));
    let first = out.split("\n\n").next().unwrap();
    assert_eq!(first, AI_DISCLOSURE);
    assert!(out.ends_with("盤點未來三年家庭可動用的現金流。"), "{out}");
}

#[test]
fn a_repaired_body_that_kept_the_disclosure_carries_it_once() {
    let once = with_disclosure(&paragraphs(BODY));
    // The repair prompt shows the model the body with the disclosure; it may
    // keep it, move it, or drop it.
    let moved = format!("{BODY}\n{AI_DISCLOSURE}");
    for reply in [once.clone(), moved] {
        let out = with_disclosure(&paragraphs(&reply));
        assert_eq!(out.matches(AI_DISCLOSURE).count(), 1, "{out}");
        assert_eq!(out, once);
    }
}

#[test]
fn the_disclosure_sits_outside_every_validated_section() {
    for marker in ["本週訊號", "對應檢視面向", "本週檢視動作"] {
        assert!(!AI_DISCLOSURE.contains(marker), "contains {marker}");
    }
    let out = with_disclosure(&paragraphs(BODY));
    let actions = &out[out.find("本週檢視動作").unwrap()..];
    assert!(!actions.contains(AI_DISCLOSURE));
}

// ── issue_title ──────────────────────────────────────────────────────────────

use liko_finance_weekly::proc::issue_title;

fn reply(title_block: &str) -> String {
    format!("先查了來源……\nBEGIN_ISSUE_TITLE\n{title_block}\nEND_ISSUE_TITLE\nBEGIN_ISSUE_BODY\n本週訊號\nEND_ISSUE_BODY")
}

#[test]
fn a_title_between_its_markers_is_taken_trimmed() {
    assert_eq!(issue_title(&reply("  連續受益人信託走向長期治理  ")).unwrap(), "連續受益人信託走向長期治理");
}

#[test]
fn one_wrapper_around_the_whole_title_is_dropped_inner_brackets_kept() {
    assert_eq!(issue_title(&reply("**連續受益人信託走向長期治理**")).unwrap(), "連續受益人信託走向長期治理");
    assert_eq!(issue_title(&reply("## 連續受益人信託走向長期治理")).unwrap(), "連續受益人信託走向長期治理");
    assert_eq!(issue_title(&reply("「連續受益人信託」成形，傳承走向長期治理")).unwrap(), "「連續受益人信託」成形，傳承走向長期治理");
    assert_eq!(issue_title(&reply("「信託」與「保單」的傳承分工")).unwrap(), "「信託」與「保單」的傳承分工");
}

#[test]
fn missing_markers_multiple_lines_and_bad_lengths_are_rejected() {
    assert!(issue_title("BEGIN_ISSUE_BODY\n本週訊號\nEND_ISSUE_BODY").is_err());
    assert!(issue_title("BEGIN_ISSUE_TITLE\n連續受益人信託走向長期治理").is_err());
    assert!(issue_title(&reply("第一行標題很長很長\n第二行標題也很長")).is_err());
    assert!(issue_title(&reply("短標題")).is_err());
    assert!(issue_title(&reply(&"財".repeat(41))).is_err());
    assert!(issue_title(&reply(&"財".repeat(40))).is_ok());
}

#[test]
fn trading_verbs_are_rejected_but_market_nouns_are_not() {
    assert!(issue_title(&reply("美債殖利率回落，現在是加碼時機")).is_err());
    assert!(issue_title(&reply("美元保單買氣升溫，先檢視幣別配置")).is_ok());
}
