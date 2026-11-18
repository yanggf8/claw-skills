//! The process-wide `ai_sources: shadow` trace stamp [R9], in-process.
//!
//! The stamp cannot be unset, so it lives in a test binary of its own: every
//! test here runs as a shadow process, and no other binary is affected.
//! `shadow_ai.rs` covers the other half — the built executable sets it, and
//! the library alone never does.

mod support;

use news::precheck::new_cache;
use news::run::LiveFeeds;
use news::shadow::run_shadow_ai;
use news::trace::{log_trace, stamp_process_as_shadow};
use support::*;

#[test]
fn every_event_is_stamped_including_one_from_a_thread_after_the_section_returns() {
    let env = Env::new();
    stamp_process_as_shadow();
    two_story_replies(&env);
    let p = publisher();
    env.write_feed_config(&[("iThome", p.url("/ai.xml"))]);

    std::env::set_var("NULLCLAW_JOB_ID", JOB);
    let mut out: Vec<u8> = Vec::new();
    let code = run_shadow_ai(&LiveFeeds, DATE, &new_cache(), &mut out);
    assert_eq!(code, 0);
    assert_eq!(String::from_utf8(out).unwrap(), markers("ok"));

    // Late, abandoned-style work: a thread born after the section returned,
    // writing with a field of the same name that the stamp must override.
    std::thread::spawn(|| {
        log_trace(
            "late_worker_event",
            serde_json::json!({"ai_sources": "direct", "n": 1}),
        )
    })
    .join()
    .unwrap();
    std::env::remove_var("NULLCLAW_JOB_ID");

    let lines = env.trace_lines();
    for l in &lines {
        assert_eq!(l.matches("\"ai_sources\"").count(), 1, "{l}");
        let v: serde_json::Value = serde_json::from_str(l).unwrap();
        assert_eq!(v["ai_sources"], "shadow", "{l}");
    }
    let late = env.traces("late_worker_event");
    assert_eq!(late.len(), 1);
    assert_eq!(late[0]["n"], 1);
    let rendered = env.traces("ai_section_rendered");
    assert_eq!(rendered.len(), 1);
    assert_eq!(rendered[0]["stories"], 2);
    assert_eq!(env.traces("ai_sources_shadow").len(), 1);
}
