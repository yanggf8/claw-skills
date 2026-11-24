//! `news shadow-ai` — spec `docs/specs/2026-11-17-news-ai-direct-feeds-design.md`
//! §6 (shadow, R2, R9) and §7 (traces).
//!
//! The process-wide trace stamp is never set in this test binary: the
//! subcommand sets it in `main`, so it is asserted here only through the
//! built executable, and an in-process shadow run followed by a delivering
//! run shows the library alone stamps nothing. `shadow_stamp.rs` is the one
//! binary that sets it in-process.

mod support;

use news::alert::{alert_failure, AlertContext};
use news::direct::{ConfigError, DirectReport};
use news::precheck::new_cache;
use news::run::{run_deliver, FeedSource, LiveFeeds};
use news::shadow::{run_shadow_ai, run_shadow_ai_with};
use news::text::Item;
use std::path::PathBuf;
use std::process::Command;
use support::*;

/// The AI section from the operator's direct feeds under `$HOME`; nothing
/// else exists. A shadow build must ask for nothing but `direct`.
#[derive(Default)]
struct DirectOnly {
    asked: std::sync::Mutex<Vec<String>>,
}

impl DirectOnly {
    fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }
}

impl FeedSource for DirectOnly {
    fn builtin(&self, name: &str) -> Vec<Item> {
        self.asked.lock().unwrap().push(name.to_string());
        Vec::new()
    }
    fn topic(&self, topic: &str) -> Vec<Item> {
        self.asked.lock().unwrap().push(format!("topic:{topic}"));
        Vec::new()
    }
    fn direct_ai(&self) -> Result<(Vec<Item>, DirectReport), ConfigError> {
        self.asked.lock().unwrap().push("direct".to_string());
        LiveFeeds.direct_ai()
    }
}

/// `(exit code, stdout)` of one scheduled in-process shadow run.
fn shadow_run(feeds: &DirectOnly) -> (i32, String) {
    std::env::set_var("NULLCLAW_JOB_ID", JOB);
    let mut out: Vec<u8> = Vec::new();
    let code = run_shadow_ai(feeds, DATE, &new_cache(), &mut out);
    std::env::remove_var("NULLCLAW_JOB_ID");
    (code, String::from_utf8(out).unwrap())
}

/// A context that WOULD alert — a chat id, a bot token on disk, and a
/// Telegram stub that records every request — handed to the shadow, which
/// must silence it.
fn loud_ctx(telegram: &Stub) -> AlertContext {
    let mut ctx = AlertContext::new(Some("777".into()), "main".into(), Some(JOB.into()));
    ctx.telegram_base_url = Some(telegram.base.clone());
    ctx
}

fn shadow_run_with(ctx: AlertContext, feeds: &DirectOnly) -> (i32, String) {
    std::env::set_var("NULLCLAW_JOB_ID", JOB);
    let mut out: Vec<u8> = Vec::new();
    let code = run_shadow_ai_with(ctx, feeds, DATE, &new_cache(), &mut out);
    std::env::remove_var("NULLCLAW_JOB_ID");
    (code, String::from_utf8(out).unwrap())
}

/// An event with the four envelope fields every trace line carries removed.
fn payload(mut v: serde_json::Value) -> serde_json::Value {
    let o = v.as_object_mut().unwrap();
    for k in ["ts", "job_id", "skill", "event"] {
        assert!(o.remove(k).is_some(), "{k} missing");
    }
    v
}

/// The `ai_sources_shadow` summary with `elapsed_ms` checked and removed.
fn shadow_summary(env: &Env) -> serde_json::Value {
    let events = env.traces("ai_sources_shadow");
    assert_eq!(events.len(), 1, "{events:?}");
    let mut p = payload(events[0].clone());
    let elapsed = p.as_object_mut().unwrap().remove("elapsed_ms");
    assert!(elapsed.as_ref().is_some_and(|e| e.is_u64()), "{elapsed:?}");
    p
}

fn silenced_reasons(env: &Env) -> Vec<String> {
    env.traces("news_alert_silenced")
        .iter()
        .map(|e| e["reason"].as_str().unwrap().to_string())
        .collect()
}

const EMPTY_SECTION: &str = "**🤖 AI 人工智慧**\n- 今日無相關新聞\n";

// ── R2: the silent alert context ─────────────────────────────────────────────

#[test]
fn a_silent_context_never_sends_even_with_a_chat_id_and_a_token() {
    let env = Env::new();
    env.write_bot_token();
    let telegram = Stub::start(|_| Vec::new());
    let ctx = loud_ctx(&telegram).silenced();
    alert_failure(&ctx, "ai_direct_feeds_empty", "every feed failed");

    assert_eq!(telegram.seen(), Vec::<String>::new());
    assert!(!env.failure_log().exists());
    assert_eq!(env.traces("news_failure_alert").len(), 0);
    let silenced: Vec<serde_json::Value> = env
        .traces("news_alert_silenced")
        .into_iter()
        .map(payload)
        .collect();
    assert_eq!(
        silenced,
        vec![serde_json::json!({
            "reason": "ai_direct_feeds_empty",
            "detail": "every feed failed",
            "detail_chars": 17
        })]
    );
}

#[test]
fn the_same_context_unsilenced_reaches_the_telegram_stub_and_the_log() {
    // The control: without it, a stub that never sees a request proves only
    // that the seam is not wired.
    let env = Env::new();
    env.write_bot_token();
    let telegram = Stub::start(|_| Vec::new());
    alert_failure(&loud_ctx(&telegram), "ai_direct_feeds_empty", "every feed failed");

    assert_eq!(telegram.seen(), vec!["POST /botTESTTOKEN/sendMessage"]);
    let log = std::fs::read_to_string(env.failure_log()).unwrap();
    assert!(log.contains("reason: ai_direct_feeds_empty\n"), "{log}");
    assert_eq!(env.traces("news_failure_alert").len(), 1);
    assert_eq!(env.traces("news_alert_silenced").len(), 0);
}

#[test]
fn a_silenced_detail_is_clipped_to_500_characters_in_the_trace() {
    let env = Env::new();
    let ctx = AlertContext::new(None, "main".into(), None).silenced();
    let long = "長".repeat(700);
    alert_failure(&ctx, "r", &long);
    let e = &env.traces("news_alert_silenced")[0];
    assert_eq!(e["detail"].as_str().unwrap(), "長".repeat(500));
    assert_eq!(e["detail_chars"], 700);
}

// ── §6 the shadow run ────────────────────────────────────────────────────────

#[test]
fn a_shadow_run_writes_the_direct_section_and_prints_only_the_ok_markers() {
    let env = Env::new();
    two_story_replies(&env);
    let p = publisher();
    env.write_feed_config(&[("iThome", p.url("/ai.xml"))]);
    let feeds = DirectOnly::default();
    let (code, out) = shadow_run(&feeds);

    assert_eq!(code, 0);
    assert_eq!(out, markers("ok"));
    assert_eq!(
        std::fs::read_to_string(env.shadow_file()).unwrap(),
        two_story_section(&p)
    );
    // Atomic: the temp file is gone once the rename lands.
    assert_eq!(env.shadow_listing(), vec![format!("{FILE_DATE}.md")]);
    // Only the direct feeds; no Google feed is ever asked for.
    assert_eq!(feeds.asked(), vec!["direct"]);
    assert_eq!(p.seen(), vec!["GET /ai.xml", "GET /art/1", "GET /art/2"]);
    assert_eq!(env.agent_calls(), 2);
    assert!(!env.failure_log().exists());
    assert_eq!(
        shadow_summary(&env),
        serde_json::json!({
            "path": env.shadow_file().display().to_string(),
            "outcome": "ok",
            "stories": 2,
            "links_total": 2,
            "links_google": 0
        })
    );
    let rendered = env.traces("ai_section_rendered");
    assert_eq!(rendered.len(), 1, "{rendered:?}");
    assert_eq!(rendered[0]["stories"], 2);
    // In-process the stamp is unset: the event carries its own mode.
    assert_eq!(rendered[0]["ai_sources"], "direct");
    // The direct cache variant, never the Google one.
    let day = env.home.join(".nullclaw/.news-cache").join(FILE_DATE);
    let mut cached: Vec<String> = std::fs::read_dir(day)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.ends_with(".txt"))
        .collect();
    cached.sort();
    let direct = news::sources::AiSources::Direct.cache_variant();
    assert_eq!(
        cached,
        vec![format!("{direct}-000-001.txt"), format!("{direct}-001-002.txt")]
    );
}

#[test]
fn a_rerun_on_the_same_day_replaces_the_file_whole() {
    let env = Env::new();
    two_story_replies(&env);
    let p = publisher();
    env.write_feed_config(&[("iThome", p.url("/ai.xml"))]);
    std::fs::create_dir_all(env.shadow_dir()).unwrap();
    std::fs::write(env.shadow_file(), "舊內容，比新內容長得多得多得多得多得多得多得多得多得多\n".repeat(20))
        .unwrap();
    let (code, _) = shadow_run(&DirectOnly::default());
    assert_eq!(code, 0);
    assert_eq!(
        std::fs::read_to_string(env.shadow_file()).unwrap(),
        two_story_section(&p)
    );
    assert_eq!(env.shadow_listing(), vec![format!("{FILE_DATE}.md")]);
}

#[test]
fn a_missing_config_writes_the_empty_section_and_reports_degraded_silently() {
    let env = Env::new();
    env.write_bot_token();
    let telegram = Stub::start(|_| Vec::new());
    let feeds = DirectOnly::default();
    let (code, out) = shadow_run_with(loud_ctx(&telegram), &feeds);

    assert_eq!(code, 0);
    assert_eq!(out, markers("degraded"));
    assert_eq!(std::fs::read_to_string(env.shadow_file()).unwrap(), EMPTY_SECTION);
    assert_eq!(feeds.asked(), vec!["direct"]);
    assert_eq!(env.agent_calls(), 0);
    // R2: alerted to the trace only.
    assert_eq!(telegram.seen(), Vec::<String>::new());
    assert!(!env.failure_log().exists());
    assert_eq!(silenced_reasons(&env), vec!["ai_direct_feeds_empty"]);
    assert_eq!(env.traces("news_failure_alert").len(), 0);
    assert_eq!(
        shadow_summary(&env),
        serde_json::json!({
            "path": env.shadow_file().display().to_string(),
            "outcome": "empty",
            "stories": 0,
            "links_total": 0,
            "links_google": 0
        })
    );
}

#[test]
fn the_public_entry_point_is_silent_too() {
    // `run_shadow_ai` is what `main` calls; it has no chat id to reach
    // Telegram with, so the observable is the failure log.
    let env = Env::new();
    let (code, out) = shadow_run(&DirectOnly::default());
    assert_eq!(code, 0);
    assert_eq!(out, markers("degraded"));
    assert!(!env.failure_log().exists());
    assert_eq!(silenced_reasons(&env), vec!["ai_direct_feeds_empty"]);
}

#[test]
fn every_feed_failing_is_degraded_and_silent() {
    let env = Env::new();
    env.write_bot_token();
    let telegram = Stub::start(|_| Vec::new());
    let p = Stub::start(|_| Vec::new());
    env.write_feed_config(&[("Dead", p.url("/gone.xml"))]);
    let (code, out) = shadow_run_with(loud_ctx(&telegram), &DirectOnly::default());

    assert_eq!(code, 0);
    assert_eq!(out, markers("degraded"));
    assert_eq!(std::fs::read_to_string(env.shadow_file()).unwrap(), EMPTY_SECTION);
    assert_eq!(telegram.seen(), Vec::<String>::new());
    assert!(!env.failure_log().exists());
    let silenced = env.traces("news_alert_silenced");
    assert_eq!(silenced.len(), 1, "{silenced:?}");
    assert_eq!(silenced[0]["reason"], "ai_direct_feeds_empty");
    assert_eq!(
        silenced[0]["detail"],
        "direct AI feeds kept 0 items; AI section empty, google not used. \
Dead=http_error(HTTP 418) fetched=0 kept=0"
    );
}

#[test]
fn level3_exhaustion_writes_a_failure_section_and_is_degraded_and_silent() {
    let env = Env::new();
    env.write_bot_token();
    let telegram = Stub::start(|_| Vec::new());
    // Every model reply is rejected; with two items each half is size 1 and
    // cannot subdivide, so the first failed half is terminal.
    env.reply_default("沒有編號的回覆");
    let p = publisher();
    env.write_feed_config(&[("iThome", p.url("/ai.xml"))]);
    let (code, out) = shadow_run_with(loud_ctx(&telegram), &DirectOnly::default());

    assert_eq!(code, 0);
    assert_eq!(out, markers("degraded"));
    assert_eq!(
        std::fs::read_to_string(env.shadow_file()).unwrap(),
        "**🤖 AI 人工智慧**\n（AI 段未產生：ai_substage_level3_failed）\n"
    );
    assert_eq!(telegram.seen(), Vec::<String>::new());
    assert!(!env.failure_log().exists());
    assert_eq!(silenced_reasons(&env), vec!["ai_substage_level3_failed"]);
    // Nothing was rendered, and the day is recorded as exhausted, not a gap.
    assert_eq!(
        env.traces("ai_section_rendered")
            .into_iter()
            .map(payload)
            .collect::<Vec<_>>(),
        vec![serde_json::json!({"stories": 0, "links_total": 0, "links_google": 0,
                                "ai_sources": "direct", "exhausted": true})]
    );
    let direct = news::sources::AiSources::Direct.cache_variant();
    assert_eq!(
        env.traces("llm_validation_failed")
            .iter()
            .map(|e| serde_json::json!([e["variant"], e["reason"], e["section"], e["range"],
                                        e["stdout_sample"]]))
            .collect::<Vec<_>>(),
        vec![
            serde_json::json!([format!("{direct}_0_1"), "marker_validation", "ai", [0, 1],
                               "沒有編號的回覆"]),
            serde_json::json!([format!("{direct}_1_2"), "marker_validation", "ai", [1, 2],
                               "沒有編號的回覆"]),
        ]
    );
    assert_eq!(
        shadow_summary(&env),
        serde_json::json!({
            "path": env.shadow_file().display().to_string(),
            "outcome": "exhausted",
            "stories": 0,
            "links_total": 0,
            "links_google": 0
        })
    );
}

#[test]
fn a_file_that_cannot_be_written_exits_one_with_no_markers() {
    let env = Env::new();
    two_story_replies(&env);
    let p = publisher();
    env.write_feed_config(&[("iThome", p.url("/ai.xml"))]);
    // The directory's name is taken by a regular file.
    std::fs::write(env.shadow_dir(), "not a directory").unwrap();
    let (code, out) = shadow_run(&DirectOnly::default());

    assert_eq!(code, 1);
    assert_eq!(out, "");
    let mut s = shadow_summary(&env);
    let error = s.as_object_mut().unwrap().remove("error");
    assert!(error.as_ref().is_some_and(|e| e.is_string()), "{error:?}");
    assert_eq!(
        s,
        serde_json::json!({
            "path": env.shadow_file().display().to_string(),
            "outcome": "write_failed",
            "stories": 2,
            "links_total": 2,
            "links_google": 0
        })
    );
}

#[test]
fn an_in_process_shadow_then_a_delivering_run_carry_no_stamp() {
    // The stamp is the subcommand's, set in `main`. Were the library to set
    // it, this delivering run — after a shadow build in the same process —
    // would be stamped.
    let env = Env::new();
    two_story_replies(&env);
    let p = publisher();
    env.write_feed_config(&[("iThome", p.url("/ai.xml"))]);
    let feeds = DirectOnly::default();
    let (code, _) = shadow_run(&feeds);
    assert_eq!(code, 0);

    std::env::set_var("NEWS_AI_SOURCES", "direct");
    std::env::set_var("NULLCLAW_JOB_ID", JOB);
    let mut out: Vec<u8> = Vec::new();
    let args = news::cli::DeliverArgs {
        lang: "zh".into(),
        account: "main".into(),
        ..Default::default()
    };
    let code = run_deliver(&args, &feeds, DATE, &new_cache(), &mut out);
    std::env::remove_var("NULLCLAW_JOB_ID");
    std::env::remove_var("NEWS_AI_SOURCES");
    assert_eq!(code, 0);

    let all = env.all_traces();
    assert!(all.iter().any(|e| e["event"] == "news_run_timing"));
    for e in &all {
        if e["event"] == "ai_section_rendered" {
            assert_eq!(e["ai_sources"], "direct", "{e}");
        } else {
            assert!(e.get("ai_sources").is_none(), "{e}");
        }
    }
    assert_eq!(env.traces("ai_section_rendered").len(), 2);
}

/// The model calls' variants, in order.
fn asked(env: &Env) -> Vec<String> {
    env.traces("llm_agent_start")
        .iter()
        .map(|e| e["variant"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn a_wrong_shaped_direct_half_is_rescued_by_one_retry_under_its_own_variant() {
    let env = Env::new();
    let cot = format!("- #1 {D1}\n讓我再想想。");
    env.reply(1, &cot);
    env.reply(2, &format!("- #1 {D1}"));
    env.reply(3, &format!("- #1 {D2}"));
    let p = publisher();
    env.write_feed_config(&[("iThome", p.url("/ai.xml"))]);
    let (code, out) = shadow_run(&DirectOnly::default());

    assert_eq!(code, 0);
    assert_eq!(out, markers("ok"));
    assert_eq!(
        std::fs::read_to_string(env.shadow_file()).unwrap(),
        two_story_section(&p)
    );
    let direct = news::sources::AiSources::Direct.cache_variant();
    assert_eq!(
        asked(&env),
        vec![
            format!("{direct}_0_1"),
            format!("{direct}_0_1_reformat"),
            format!("{direct}_1_2"),
        ]
    );
    assert_eq!(
        env.traces("section_reformat_retry_ok")
            .into_iter()
            .map(payload)
            .collect::<Vec<_>>(),
        vec![serde_json::json!({"section": "ai", "range": [0, 1], "attempt": 2,
                                "first_sample": cot})]
    );
    assert_eq!(env.traces("llm_validation_failed").len(), 0);
}

// ── the built executable ─────────────────────────────────────────────────────

fn taipei_date() -> String {
    jiff::Timestamp::now()
        .in_tz("Asia/Taipei")
        .unwrap()
        .strftime("%Y-%m-%d")
        .to_string()
}

fn shadow_binary(env: &Env, args: &[&str]) -> std::process::Output {
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_news")));
    cmd.args(args).env("HOME", &env.home).env("NULLCLAW_JOB_ID", JOB);
    for k in CLEARED.iter().filter(|k| **k != "NULLCLAW_JOB_ID") {
        cmd.env_remove(k);
    }
    for (k, v) in KNOBS {
        cmd.env(k, v);
    }
    cmd.output().expect("run news")
}

#[test]
fn the_binary_stamps_every_event_shadow_and_prints_only_the_markers() {
    let env = Env::new();
    two_story_replies(&env);
    let p = publisher();
    env.write_feed_config(&[("iThome", p.url("/ai.xml"))]);
    let before = taipei_date();
    let out = shadow_binary(&env, &["shadow-ai"]);
    let after = taipei_date();

    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(String::from_utf8_lossy(&out.stdout), markers("ok"));
    let listing = env.shadow_listing();
    assert_eq!(listing.len(), 1, "{listing:?}");
    assert!(
        listing[0] == format!("{before}.md") || listing[0] == format!("{after}.md"),
        "{listing:?}"
    );
    assert_eq!(
        std::fs::read_to_string(env.shadow_dir().join(&listing[0])).unwrap(),
        two_story_section(&p)
    );
    assert!(!env.failure_log().exists());

    let lines = env.trace_lines();
    assert!(!lines.is_empty());
    for l in &lines {
        // One key: `ai_section_rendered`'s own field is overwritten, never
        // written twice.
        assert_eq!(l.matches("\"ai_sources\"").count(), 1, "{l}");
        let v: serde_json::Value = serde_json::from_str(l).unwrap();
        assert_eq!(v["ai_sources"], "shadow", "{l}");
        assert_eq!(v["job_id"], JOB, "{l}");
    }
    for event in ["ai_direct_feeds", "ai_section_rendered", "ai_sources_shadow"] {
        assert_eq!(env.traces(event).len(), 1, "{event}");
    }
}

#[test]
fn the_binary_rejects_delivery_flags_with_exit_two_before_doing_anything() {
    let env = Env::new();
    for args in [
        &["shadow-ai", "--deliver-to", "123"][..],
        &["shadow-ai", "--account", "nunu"][..],
        &["shadow-ai", "--nope"][..],
    ] {
        let out = shadow_binary(&env, args);
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        assert_eq!(String::from_utf8_lossy(&out.stdout), "", "{args:?}");
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("unknown argument"),
            "{args:?}"
        );
    }
    assert!(!env.shadow_dir().exists());
    assert_eq!(env.trace_lines(), Vec::<String>::new());
}
