//! The same-story gate for a paywalled pick's free replacement.
//!
//! The pair in `the_wired_technews_pair_is_undecidable` is the real one that
//! exposed the defect on 2026-08-12, kept verbatim: TechNews cites the Wired
//! article as its source, and the token gate still scored them 0.

use news::precheck::{headline_for, same_story_gate, StoryGate};
use news::summarize::cross_language_verdict;
use std::cell::RefCell;

const WIRED: &str = "A New Trick Reveals AI Models' Inner Thoughts - Wired";
const TECHNEWS: &str =
    "AI「內心戲」全曝光？新技術破解大模型推理軌跡，爆機密外洩隱憂 - TechNews 科技新報";

// ── the gate ─────────────────────────────────────────────────────────────────

#[test]
fn the_wired_technews_pair_is_undecidable() {
    // Not `Same`, and — this is the fix — not `Different` either. Scoring it
    // `Different` is what dropped the one free article that covered the story.
    assert_eq!(same_story_gate(WIRED, TECHNEWS), StoryGate::Undecidable);
    assert_eq!(same_story_gate(TECHNEWS, WIRED), StoryGate::Undecidable);
}

#[test]
fn two_english_headlines_about_one_event_are_the_same_story() {
    assert_eq!(
        same_story_gate(
            "Nvidia Reports Record Quarterly Revenue - Reuters",
            "Nvidia posts record revenue on AI demand - CNBC"
        ),
        StoryGate::Same
    );
}

#[test]
fn two_chinese_headlines_about_one_event_are_the_same_story() {
    assert_eq!(
        same_story_gate(
            "台積電法說會上修全年展望 - 工商時報",
            "台積電法說會釋利多 上修全年營收展望 - 經濟日報"
        ),
        StoryGate::Same
    );
}

#[test]
fn same_script_but_unrelated_is_still_rejected_deterministically() {
    // The cheap reject has to survive the change: without it every unrelated
    // candidate would cost a network precheck and possibly a model call.
    assert_eq!(
        same_story_gate(
            "A New Trick Reveals AI Models' Inner Thoughts - Wired",
            "Fed holds rates steady as inflation cools - AP"
        ),
        StoryGate::Different
    );
    assert_eq!(
        same_story_gate("台積電法說會上修全年展望 - 工商時報", "颱風假明天停班停課 - 自由時報"),
        StoryGate::Different
    );
}

#[test]
fn an_english_headline_from_a_chinese_outlet_is_not_mistaken_for_chinese() {
    // The source suffix is Chinese, the headline is not. Counting the suffix
    // would call the pair cross-script and spend a model call on a question
    // the token gate can answer for free — here, correctly, `Same`.
    assert_eq!(
        same_story_gate(WIRED, "New trick reveals what AI models are thinking - 自由時報"),
        StoryGate::Same
    );
}

// ── the dispatch ─────────────────────────────────────────────────────────────

/// Records which callback was asked, so a test can prove the routing.
#[derive(Default)]
struct Calls {
    translated: RefCell<Vec<String>>,
    judged: RefCell<Vec<(String, String)>>,
}

fn run(gate: StoryGate, answer: Option<&str>) -> (Option<String>, Calls) {
    let calls = Calls::default();
    let translate = |t: &str, _d: &str| {
        calls.translated.borrow_mut().push(t.to_string());
        answer.map(str::to_string)
    };
    let judge = |o: &str, c: &str, _d: &str| {
        calls
            .judged
            .borrow_mut()
            .push((o.to_string(), c.to_string()));
        answer.map(str::to_string)
    };
    let out = headline_for(gate, WIRED, TECHNEWS, "2026-08-13", &translate, &judge);
    (out, calls)
}

#[test]
fn an_undecidable_pair_goes_to_the_judge_and_never_to_the_translator() {
    let (out, calls) = run(StoryGate::Undecidable, Some("AI 內心戲全曝光"));
    assert_eq!(out.as_deref(), Some("AI 內心戲全曝光"));
    assert!(calls.translated.borrow().is_empty(), "translator was called");
    // Both titles must reach the judge: with only the candidate it cannot
    // answer the equivalence question at all.
    assert_eq!(
        *calls.judged.borrow(),
        vec![(WIRED.to_string(), TECHNEWS.to_string())]
    );
}

#[test]
fn a_same_story_pair_goes_to_the_translator_and_never_to_the_judge() {
    let (out, calls) = run(StoryGate::Same, Some("輝達營收創高"));
    assert_eq!(out.as_deref(), Some("輝達營收創高"));
    assert_eq!(*calls.translated.borrow(), vec![TECHNEWS.to_string()]);
    assert!(calls.judged.borrow().is_empty(), "judge was called");
}

#[test]
fn a_different_pair_asks_no_model_at_all() {
    let (out, calls) = run(StoryGate::Different, Some("不該出現"));
    assert!(out.is_none());
    assert!(calls.translated.borrow().is_empty());
    assert!(calls.judged.borrow().is_empty());
}

#[test]
fn a_judge_that_refuses_rejects_the_candidate() {
    let (out, calls) = run(StoryGate::Undecidable, None);
    assert!(out.is_none());
    assert_eq!(calls.judged.borrow().len(), 1);
}

#[test]
fn a_blank_headline_is_not_a_replacement() {
    // A whitespace-only answer would render as a bare link with no text.
    assert!(run(StoryGate::Undecidable, Some("   ")).0.is_none());
    assert!(run(StoryGate::Same, Some("")).0.is_none());
}

// ── parsing the judge's answer ───────────────────────────────────────────────

#[test]
fn a_refusal_is_recognised_however_it_is_cased_or_bulleted() {
    assert!(cross_language_verdict("NO").is_none());
    assert!(cross_language_verdict("no").is_none());
    assert!(cross_language_verdict("- NO\n").is_none());
    assert!(cross_language_verdict("\n\n  No  \n").is_none());
    assert!(cross_language_verdict("").is_none());
}

#[test]
fn a_chinese_headline_is_taken_from_the_first_non_empty_line() {
    assert_eq!(
        cross_language_verdict("\n- AI「內心戲」全曝光？新技術破解大模型推理軌跡\n多餘的第二行"),
        Some("AI「內心戲」全曝光？新技術破解大模型推理軌跡".to_string())
    );
}

#[test]
fn an_english_answer_is_refused_even_though_it_is_not_the_word_no() {
    // The model echoing the English original is the observed way this goes
    // wrong, and it arrives after the section language gate has already run,
    // so nothing downstream would catch it.
    assert!(cross_language_verdict("A New Trick Reveals AI Models' Inner Thoughts").is_none());
    assert!(cross_language_verdict("- Yes, they are the same event.").is_none());
}

#[test]
fn a_markdown_link_is_unwrapped_to_its_text() {
    assert_eq!(
        cross_language_verdict("- [台積電法說會上修全年展望](https://example.com/a)"),
        Some("台積電法說會上修全年展望".to_string())
    );
}
