//! The two agent passes, and what must never reach a reader.
//!
//! Ported from `mindfulness-spirit/scripts/test_run.py`, whose docstrings still
//! call themselves RED-phase although the behaviour they demanded has long
//! since landed. The claims are what matter and they are kept here: a failed
//! checklist aborts before publish, a degraded run never marks itself
//! validation-ok, harness tokens never reach either the reviewer or the body,
//! and paragraph blank lines survive.

use mindfulness_spirit::agent::{Output, TIMEOUT_RC};
use mindfulness_spirit::pipeline::{write_and_review, Draft};
use std::cell::RefCell;

fn ok(stdout: &str) -> Output {
    Output {
        code: 0,
        stdout: stdout.to_string(),
        stderr: String::new(),
    }
}

fn fail(code: i32, stderr: &str) -> Output {
    Output {
        code,
        stdout: String::new(),
        stderr: stderr.to_string(),
    }
}

/// Serves one scripted reply per call and records every prompt it was given.
struct Agent {
    replies: RefCell<Vec<Output>>,
    prompts: RefCell<Vec<String>>,
}

impl Agent {
    fn new(replies: Vec<Output>) -> Agent {
        Agent {
            replies: RefCell::new(replies),
            prompts: RefCell::new(Vec::new()),
        }
    }
    fn call(&self, prompt: &str) -> Output {
        self.prompts.borrow_mut().push(prompt.to_string());
        let mut r = self.replies.borrow_mut();
        if r.is_empty() {
            // Fail closed: an unscripted call is the test not matching the
            // code, and must not read as a pass.
            return fail(99, "unscripted agent call");
        }
        r.remove(0)
    }
    fn calls(&self) -> usize {
        self.prompts.borrow().len()
    }
    fn prompt(&self, i: usize) -> String {
        self.prompts.borrow()[i].clone()
    }
}

fn run(replies: Vec<Output>) -> (Draft, Agent, String, String) {
    let agent = Agent::new(replies);
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let draft = write_and_review(
        "WRITER PROMPT",
        "CHECKLIST\n{{WRITER_OUTPUT}}\nEND",
        &|p| agent.call(p),
        &mut out,
        &mut err,
    );
    (
        draft,
        agent,
        String::from_utf8(out).unwrap(),
        String::from_utf8(err).unwrap(),
    )
}

#[test]
fn a_clean_pair_of_passes_yields_a_reviewed_body() {
    let (draft, agent, _, _) = run(vec![ok("乾淨草稿"), ok("乾淨正文。")]);
    assert_eq!(agent.calls(), 2);
    match draft {
        Draft::Reviewed {
            body,
            validation_summary,
        } => {
            assert_eq!(body, "乾淨正文。");
            assert_eq!(validation_summary, "checklist passed");
        }
        Draft::Failed { reason, .. } => panic!("expected a reviewed draft: {reason}"),
    }
}

#[test]
fn a_failed_checklist_aborts_rather_than_shipping_the_draft() {
    // The tempting alternative — publish the unreviewed writer output and
    // label it degraded — makes the label the only difference between
    // reviewed and not.
    let (draft, _, _, err) = run(vec![ok("文章內容"), fail(1, "boom")]);
    match draft {
        Draft::Failed { code, reason } => {
            assert_ne!(code, 0);
            assert!(reason.contains("checklist phase degraded"), "{reason}");
        }
        Draft::Reviewed { .. } => panic!("an unreviewed draft was accepted"),
    }
    assert!(err.contains("[checklist] degraded"), "{err}");
}

#[test]
fn a_checklist_timeout_aborts_the_same_way_as_a_refusal() {
    let (draft, _, _, _) = run(vec![ok("文章內容"), fail(TIMEOUT_RC, "")]);
    match draft {
        Draft::Failed { code, reason } => {
            assert_eq!(code, TIMEOUT_RC);
            assert!(reason.contains("timed out after 300s"), "{reason}");
        }
        Draft::Reviewed { .. } => panic!("a timed-out review was accepted"),
    }
}

#[test]
fn a_failed_writer_never_reaches_the_checklist() {
    let (draft, agent, _, err) = run(vec![fail(1, "boom")]);
    assert_eq!(agent.calls(), 1, "the reviewer was asked about nothing");
    assert!(matches!(draft, Draft::Failed { code: 1, .. }));
    assert!(err.contains("writer agent failed"), "{err}");
}

#[test]
fn harness_tokens_never_reach_the_reviewer() {
    // The reviewer is asked to judge prose. A leaked protocol block is
    // something it would try to reason about.
    let (_, agent, _, _) = run(vec![
        ok("好文章\n\n<ncchoices>{\"v\":1}</ncchoices>"),
        ok("通過"),
    ]);
    let checklist_prompt = agent.prompt(1);
    assert!(!checklist_prompt.contains("<ncchoices>"), "{checklist_prompt}");
    assert!(checklist_prompt.contains("好文章"));
}

#[test]
fn harness_tokens_never_reach_the_body_but_paragraph_breaks_do() {
    // Sanitising in chat mode would collapse the blank lines, and in an
    // article they are the paragraph separators.
    let (draft, _, _, _) = run(vec![
        ok("草稿"),
        ok("第一段。\n\n<ncchoices>{\"v\":1}</ncchoices>\n\n第二段。"),
    ]);
    let Draft::Reviewed { body, .. } = draft else {
        panic!("expected a reviewed draft");
    };
    assert!(!body.contains("ncchoices"), "{body}");
    assert!(body.starts_with("第一段。"), "{body}");
    assert!(body.ends_with("第二段。"), "{body}");
    assert!(body.contains("\n\n"), "paragraph break lost: {body:?}");
}

#[test]
fn an_unclosed_protocol_block_is_stripped_too() {
    // The model drops the closing tag often enough that only handling the
    // paired form leaves the common case in the article.
    let (draft, _, _, _) = run(vec![ok("草稿"), ok("正文。\n<ncchoices>{\"v\":1")]);
    let Draft::Reviewed { body, .. } = draft else {
        panic!("expected a reviewed draft");
    };
    assert_eq!(body, "正文。");
}
