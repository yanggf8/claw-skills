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
