//! The section pipeline, end to end, against a stub agent.
//!
//! Ported from the orchestration half of `news/scripts/test_run.py`, which
//! fakes the agent with `patch.object(run, "_run_nullclaw_agent", ...)`. Rust
//! has no monkeypatch, so the seam is the one `claw-core` already uses: the
//! agent binary is resolved through `$HOME`, and these tests point `$HOME` at a
//! scratch directory holding a shell script that serves canned replies. That
//! buys more than a patched function would — the real subprocess handling, the
//! pipe draining, the timeout kill and the exit-code branch all run.
//!
//! Assertions read the trace file the skill actually writes, rather than
//! intercepting calls. Ordering claims like "the collapse runs before the
//! precheck" then hold against real execution instead of against a recorded
//! call list.
//!
//! Everything is offline. Article links point at `127.0.0.1:1`, so the precheck
//! decode fails instantly with connection refused and the item is kept
//! unresolved — the same verdict it would reach on a machine with no network.
//!
//! No test here writes a quality config. `quality::active_config` is a
//! process-wide `OnceLock` — right for production, where one run reads one
//! config — which means the first test to touch it fixes it for every test in
//! the same binary. A deny list belongs in its own binary; see `tests/deny.rs`.

use news::agent::run_agent;
use news::alert::AlertContext;
use news::precheck::new_cache;
use news::render::LinkMap;
use news::select::NumberedMap;
use news::summarize::{run_ai_substage, run_custom_topic, summarize_default_section};
use news::text::Item;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Mutex;

/// `$HOME`, the stub's response queue and the trace file are all process-wide,
/// so the tests in this file run one at a time.
static LOCK: Mutex<()> = Mutex::new(());

const DATE: &str = "2026/07/13 (Mon)";

/// Five real headlines: 1 and 5 are the same story from two outlets, 2 is a
/// related but distinct one, 3 and 4 are unrelated. The overlaps are what the
/// hint threshold and the collapse were tuned against.
const TECH_FIXTURE: [&str; 5] = [
    "川普「晶片回流」恐夢碎？專家揭美半導體業最大危機：台積電、美光、三星都難逃",
    "美光、三星領跌 記憶體晶片股集體陷技術性熊市",
    "半導體股震盪何時完結？",
    "晶片股慘遭拋售 輝達選擇權卻爆量押多 甦醒時刻到了？",
    "台積電也難逃！彭博爆美國晶片業「拉警報」 最大危機曝光",
];

fn items(titles: &[&str]) -> Vec<Item> {
    titles
        .iter()
        .enumerate()
        .map(|(i, t)| Item {
            title: (*t).to_string(),
            source: "src".to_string(),
            // Unroutable on purpose: the precheck decode is refused instantly.
            link: format!("http://127.0.0.1:1/{}", i + 1),
            ..Default::default()
        })
        .collect()
}

struct Env {
    home: PathBuf,
}

impl Env {
    /// A scratch HOME with a stub agent installed.
    fn new() -> Env {
        let home = std::env::temp_dir().join(format!("news-pipe-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join(".nullclaw")).unwrap();
        std::fs::create_dir_all(home.join("stub")).unwrap();
        let bin = home.join("nullclaw/zig-out/bin");
        std::fs::create_dir_all(&bin).unwrap();

        // Serves resp.<n> for the nth call, recording each prompt so tests can
        // assert on what the model was actually asked. rc.<n> and sleep.<n>
        // drive the failure and timeout branches.
        std::fs::write(
            bin.join("nullclaw"),
            r#"#!/bin/sh
d="$HOME/stub"
n=$(cat "$d/counter" 2>/dev/null || echo 0)
n=$((n + 1))
echo "$n" > "$d/counter"
printf '%s' "$4" > "$d/prompt.$n"
[ -f "$d/sleep.$n" ] && sleep "$(cat "$d/sleep.$n")"
if [ -f "$d/resp.$n" ]; then cat "$d/resp.$n"
elif [ -f "$d/resp.default" ]; then cat "$d/resp.default"
fi
exit "$(cat "$d/rc.$n" 2>/dev/null || echo 0)"
"#,
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(bin.join("nullclaw"), std::fs::Permissions::from_mode(0o755))
                .unwrap();
        }

        std::env::set_var("HOME", &home);
        std::env::remove_var("NULLCLAW_JOB_ID");
        std::env::remove_var("NULLCLAW_SKILL_TIMEOUT");
        std::env::remove_var("NULLCLAW_SKILL_STARTED");
        // Defaults for the pipeline: precheck on (it is offline here), paywall
        // lookup off (it would fetch replacement feeds), theming off.
        std::env::set_var("NEWS_PRECHECK", "1");
        std::env::set_var("NEWS_PRECHECK_DECODE_TIMEOUT", "0.05");
        std::env::set_var("NEWS_PRECHECK_FETCH_TIMEOUT", "0.05");
        std::env::set_var("NEWS_PAYWALL_REPLACE", "0");
        std::env::set_var("NEWS_AI_THEME", "off");
        std::env::set_var("NEWS_CROSS_DEDUP", "0");
        std::env::set_var("NEWS_LLM_DEDUP_HINTS", "1");
        std::env::set_var("NEWS_LLM_POST_DEDUP", "1");
        Env { home }
    }

    fn reply(&self, n: usize, stdout: &str) -> &Env {
        std::fs::write(self.home.join(format!("stub/resp.{n}")), stdout).unwrap();
        self
    }

    fn reply_default(&self, stdout: &str) -> &Env {
        std::fs::write(self.home.join("stub/resp.default"), stdout).unwrap();
        self
    }

    fn exit_code(&self, n: usize, rc: i32) -> &Env {
        std::fs::write(self.home.join(format!("stub/rc.{n}")), rc.to_string()).unwrap();
        self
    }

    fn stall(&self, n: usize, secs: &str) -> &Env {
        std::fs::write(self.home.join(format!("stub/sleep.{n}")), secs).unwrap();
        self
    }

    fn calls(&self) -> usize {
        std::fs::read_to_string(self.home.join("stub/counter"))
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0)
    }

    fn prompt(&self, n: usize) -> String {
        std::fs::read_to_string(self.home.join(format!("stub/prompt.{n}"))).unwrap_or_default()
    }

    /// Every trace event this run wrote, in order.
    fn traces(&self) -> Vec<Value> {
        std::fs::read_to_string(self.home.join(".nullclaw/skill-traces.jsonl"))
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect()
    }

    fn events(&self) -> Vec<String> {
        self.traces()
            .iter()
            .filter_map(|e| e["event"].as_str().map(str::to_string))
            .collect()
    }

    fn first(&self, event: &str) -> Option<Value> {
        self.traces().into_iter().find(|e| e["event"] == event)
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

fn ids(v: &Value, key: &str) -> Vec<u64> {
    v[key]
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_u64).collect())
        .unwrap_or_default()
}

// ── the default section ──────────────────────────────────────────────────────

#[test]
fn a_section_prompt_carries_the_dedup_rules_and_the_pair_hints() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let env = Env::new();
    env.reply_default("- #1 台積電法說會\n- #2 輝達財報亮眼\n- #3 記憶體股走弱");
    summarize_default_section("tech", &items(&TECH_FIXTURE), DATE, &LinkMap::default(), &new_cache());

    let p = env.prompt(1);
    assert!(p.contains("重複判斷以「事件本身」為準"), "dedup rules missing");
    assert!(p.contains("英文標題必須完整翻譯成繁體中文"), "translation rules missing");
    // 1 and 5 are the same story, so the hint must name that pair.
    assert!(p.contains("可能同事件候選"), "hint block missing:\n{p}");
    assert!(p.contains("#1+#5"), "expected the 1/5 hint:\n{p}");
    // Every candidate is offered, numbered.
    for n in 1..=5 {
        assert!(p.contains(&format!("#{n} ")), "candidate {n} missing");
    }
}

#[test]
fn hints_can_be_switched_off_without_touching_the_rest_of_the_prompt() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let env = Env::new();
    std::env::set_var("NEWS_LLM_DEDUP_HINTS", "0");
    env.reply_default("- #1 台積電法說會");
    summarize_default_section("tech", &items(&TECH_FIXTURE), DATE, &LinkMap::default(), &new_cache());
    std::env::set_var("NEWS_LLM_DEDUP_HINTS", "1");

    let p = env.prompt(1);
    assert!(!p.contains("可能同事件候選"), "hints leaked in:\n{p}");
    assert!(p.contains("重複判斷以「事件本身」為準"));
    let hints = env.first("llm_dedup_hints").expect("trace written");
    assert_eq!(hints["enabled"], false);
}

#[test]
fn the_collapse_runs_before_the_precheck_and_the_precheck_only_sees_survivors() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let env = Env::new();
    // The model picks 1, 2 and 5; 5 is the same story as 1.
    env.reply_default("- #1 美半導體危機影響台積電美光三星\n- #2 記憶體股技術性熊市\n- #5 彭博爆美國晶片業拉警報");
    let (lines, used_fallback) = summarize_default_section(
        "tech",
        &items(&TECH_FIXTURE),
        DATE,
        &LinkMap::default(),
        &new_cache(),
    );
    assert!(!used_fallback);

    let events = env.events();
    let collapse = events.iter().position(|e| e == "llm_post_dedup").expect("collapse ran");
    let precheck = events.iter().position(|e| e == "quality_tier2").expect("precheck ran");
    assert!(
        collapse < precheck,
        "the precheck decoded ids the collapse was about to drop: {events:?}"
    );

    let pd = env.first("llm_post_dedup").unwrap();
    assert_eq!(ids(&pd, "before"), vec![1, 2, 5]);
    assert_eq!(ids(&pd, "after"), vec![1, 2]);
    assert_eq!(ids(&pd, "dropped"), vec![5]);

    // pick "3-5" floors at three, so the refill adds a never-selected item.
    let refill = env.first("post_dedup_refill").expect("refill ran");
    assert_eq!(refill["final_count"], 3);
    // Three items reached the precheck, not the four the model plus refill
    // would imply if the collapse had not run first.
    assert_eq!(env.first("quality_tier2").unwrap()["checked"], 3);

    let body = lines.join("\n");
    assert!(!body.contains("彭博爆美國晶片業拉警報"), "duplicate delivered: {body}");
}

#[test]
fn a_reply_with_an_unmarked_line_falls_back_to_the_raw_listing() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let env = Env::new();
    env.reply_default("- #1 台積電法說會\n- 這行沒有編號");
    let (lines, used_fallback) = summarize_default_section(
        "tech",
        &items(&TECH_FIXTURE),
        DATE,
        &LinkMap::default(),
        &new_cache(),
    );
    assert!(used_fallback, "an unmarked bullet must not ship: {lines:?}");
    assert_eq!(
        env.first("llm_validation_failed").unwrap()["reason"],
        "marker_validation"
    );
    // The fallback still delivers something readable.
    assert!(lines.iter().any(|l| l.contains("川普")), "{lines:?}");
}

#[test]
fn an_empty_reply_falls_back_and_says_why() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let env = Env::new();
    env.reply_default("");
    let (_, used_fallback) = summarize_default_section(
        "tech",
        &items(&TECH_FIXTURE),
        DATE,
        &LinkMap::default(),
        &new_cache(),
    );
    assert!(used_fallback);
    assert_eq!(
        env.first("llm_validation_failed").unwrap()["reason"],
        "empty_stdout"
    );
}

#[test]
fn an_english_reply_is_sent_back_for_translation_before_it_ships() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let env = Env::new();
    env.reply(1, "- #1 TSMC crisis deepens\n- #2 Memory stocks enter bear market")
        .reply(2, "- #1 台積電危機加深\n- #2 記憶體股進入熊市");
    let (lines, used_fallback) = summarize_default_section(
        "tech",
        &items(&TECH_FIXTURE),
        DATE,
        &LinkMap::default(),
        &new_cache(),
    );
    assert!(!used_fallback);
    assert_eq!(env.calls(), 2, "the translation pass did not run");
    assert!(env.prompt(2).contains("新聞標題翻譯編輯"), "{}", env.prompt(2));
    let body = lines.join("\n");
    assert!(body.contains("台積電危機加深"), "{body}");
    assert!(!body.contains("TSMC crisis"), "English shipped: {body}");
}

#[test]
fn a_failed_translation_falls_back_rather_than_shipping_english() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let env = Env::new();
    env.reply(1, "- #1 TSMC crisis deepens\n- #2 Memory stocks enter bear market")
        .reply(2, "still english, no markers");
    let (_, used_fallback) = summarize_default_section(
        "tech",
        &items(&TECH_FIXTURE),
        DATE,
        &LinkMap::default(),
        &new_cache(),
    );
    assert!(used_fallback, "half-English prose must never ship");
    let reasons: Vec<&str> = env
        .traces()
        .iter()
        .filter(|e| e["event"] == "llm_validation_failed")
        .filter_map(|e| e["reason"].as_str().map(str::to_string))
        .map(|s| Box::leak(s.into_boxed_str()) as &str)
        .collect();
    assert!(reasons.contains(&"language_validation"), "{reasons:?}");
    assert!(reasons.contains(&"translation_retry_validation"), "{reasons:?}");
}

// ── the agent's retry rule ───────────────────────────────────────────────────
//
// Driven through `run_agent` directly rather than through a section, because
// the section timeouts are constants (90s, 60s) and a test that waits one out
// costs a minute and a half of wall clock to learn nothing extra. The timeout
// is a parameter here, so the same branches run in a second.

#[test]
fn a_stalled_call_is_retried_once_on_a_shorter_budget() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let env = Env::new();
    std::env::set_var("NEWS_LLM_RETRY_TIMEOUT", "1");
    env.stall(1, "30").reply(2, "- #1 台積電法說會");

    let r = run_agent("p", 1, "v", &[], &NumberedMap::new());
    assert_eq!(r.returncode, 0, "the retry's answer should have been used");
    assert_eq!(r.stdout.trim(), "- #1 台積電法說會");
    assert_eq!(env.calls(), 2, "expected exactly two attempts");

    let retry = env.first("llm_agent_retry").expect("retry trace");
    assert_eq!(retry["attempt"], 2);
    assert_eq!(retry["first_timeout"], 1);
    // Shorter than the attempt that just stalled: a wedged provider must not
    // be able to spend the same budget twice.
    assert_eq!(retry["retry_timeout"], 1);
    assert!(env.first("llm_agent_timeout").is_some(), "the stall was not recorded");
}

#[test]
fn the_retry_never_gets_more_time_than_the_first_attempt() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let env = Env::new();
    // The configured retry budget is larger than the original call's, and must
    // be clamped down to it rather than up.
    std::env::set_var("NEWS_LLM_RETRY_TIMEOUT", "300");
    env.stall(1, "30").reply(2, "- #1 台積電法說會");
    run_agent("p", 1, "v", &[], &NumberedMap::new());
    assert_eq!(env.first("llm_agent_retry").unwrap()["retry_timeout"], 1);
}

#[test]
fn a_plain_failure_is_not_retried() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let env = Env::new();
    // Non-zero and not a timeout: deterministic, so a second attempt would
    // spend the budget for the same non-answer.
    env.exit_code(1, 1).reply_default("- #1 台積電法說會");
    let r = run_agent("p", 5, "v", &[], &NumberedMap::new());
    assert_eq!(r.returncode, 1);
    assert_eq!(env.calls(), 1);
    assert!(env.first("llm_agent_retry").is_none(), "a plain failure was retried");
}

#[test]
fn an_empty_answer_is_not_retried_either() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let env = Env::new();
    env.reply_default("");
    let r = run_agent("p", 5, "v", &[], &NumberedMap::new());
    assert_eq!(r.returncode, 0);
    assert!(!r.usable(), "empty stdout must not count as usable");
    assert_eq!(env.calls(), 1);
}

#[test]
fn the_retry_is_skipped_when_the_cron_budget_cannot_fit_it() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let env = Env::new();
    std::env::set_var("NEWS_LLM_RETRY_TIMEOUT", "30");
    // Two seconds of budget cannot hold a thirty-second retry, so a wedged
    // provider must not be given a second chance to overrun the kill window.
    std::env::set_var("NULLCLAW_SKILL_TIMEOUT", "2");
    env.stall(1, "30");
    run_agent("p", 1, "v", &[], &NumberedMap::new());
    std::env::remove_var("NULLCLAW_SKILL_TIMEOUT");

    assert_eq!(env.calls(), 1);
    let skipped = env
        .first("llm_agent_retry_skipped_budget")
        .expect("the skip must be recorded, not silent");
    assert_eq!(skipped["retry_timeout"], 1, "clamped to the first attempt budget");
}

#[test]
fn a_missing_agent_binary_is_not_mistaken_for_a_stall() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let env = Env::new();
    std::fs::remove_file(env.home.join("nullclaw/zig-out/bin/nullclaw")).unwrap();
    let r = run_agent("p", 5, "v", &[], &NumberedMap::new());
    assert!(!r.timed_out(), "a spawn failure must not be retried as a timeout");
    assert_eq!(env.calls(), 0);
    assert!(env.first("llm_agent_spawn_error").is_some());
}

// ── the AI section, in batches ───────────────────────────────────────────────

#[test]
fn a_substage_collapses_duplicates_before_the_precheck_too() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let env = Env::new();
    let batch = items(&[TECH_FIXTURE[0], TECH_FIXTURE[4], TECH_FIXTURE[1]]);
    env.reply_default("- #1 美半導體危機影響台積電\n- #2 彭博爆美國晶片業拉警報\n- #3 記憶體股技術性熊市");

    let lines = run_ai_substage(&batch, 0, 3, DATE, &new_cache()).expect("substage ok");
    let pd = env.first("llm_post_dedup").unwrap();
    assert_eq!(ids(&pd, "dropped"), vec![2], "the same story survived twice");
    let body = lines.join("\n");
    assert!(!body.contains("彭博爆"), "{body}");
}

#[test]
fn a_substage_result_is_cached_and_the_next_call_asks_nothing() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let env = Env::new();
    let batch = items(&TECH_FIXTURE[..3]);
    env.reply_default("- #1 台積電法說會\n- #2 輝達財報亮眼");

    let first = run_ai_substage(&batch, 0, 3, DATE, &new_cache()).unwrap();
    let after_first = env.calls();
    let second = run_ai_substage(&batch, 0, 3, DATE, &new_cache()).unwrap();

    assert_eq!(first, second);
    assert_eq!(env.calls(), after_first, "the cached range was re-asked");
    assert!(env.first("news_cache_hit").is_some());
}

#[test]
fn a_substage_reports_the_shape_of_its_failure() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let env = Env::new();
    env.exit_code(1, 7).reply_default("");
    let err = run_ai_substage(&items(&TECH_FIXTURE[..3]), 0, 3, DATE, &new_cache()).unwrap_err();
    assert_eq!(err, "exit_code=7", "the driver needs the reason to log it");
}

// ── custom topics ────────────────────────────────────────────────────────────

#[test]
fn a_custom_topic_prompt_names_the_topic_and_carries_the_shared_rules() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let env = Env::new();
    env.reply_default("- #1 台積電法說會\n- #2 輝達財報亮眼");
    run_custom_topic("台積電", &items(&TECH_FIXTURE), DATE, &new_cache()).unwrap();

    let p = env.prompt(1);
    assert!(p.contains("關於「台積電」的候選新聞標題"), "{p}");
    assert!(p.contains("重複判斷以「事件本身」為準"));
    assert!(p.contains("如果今日無相關新聞"), "the placeholder instruction is missing");
}

#[test]
fn a_custom_topic_with_no_items_answers_the_placeholder_without_asking() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let env = Env::new();
    let out = run_custom_topic("台積電", &[], DATE, &new_cache()).unwrap();
    assert_eq!(out, vec!["- 今日無相關新聞"]);
    assert_eq!(env.calls(), 0, "an empty topic must not cost a model call");
}

#[test]
fn a_custom_topic_result_is_cached_per_topic() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let env = Env::new();
    env.reply_default("- #1 台積電法說會\n- #2 輝達財報亮眼");
    let first = run_custom_topic("台積電", &items(&TECH_FIXTURE), DATE, &new_cache()).unwrap();
    let calls = env.calls();
    let second = run_custom_topic("台積電", &items(&TECH_FIXTURE), DATE, &new_cache()).unwrap();
    assert_eq!(first, second);
    assert_eq!(env.calls(), calls, "the cached topic was re-asked");

    // A different topic is a different cache key.
    run_custom_topic("記憶體", &items(&TECH_FIXTURE), DATE, &new_cache()).unwrap();
    assert!(env.calls() > calls);
}

#[test]
fn a_custom_topic_reports_a_hard_failure_to_its_caller() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let env = Env::new();
    // The driver turns this into a raw listing plus an alert, so the reason
    // has to survive the call rather than being swallowed.
    env.exit_code(1, 3).reply_default("");
    let err = run_custom_topic("台積電", &items(&TECH_FIXTURE), DATE, &new_cache()).unwrap_err();
    assert_eq!(err, "exit_code=3");
}

// ── the alert context ────────────────────────────────────────────────────────

#[test]
fn an_alert_records_a_block_on_disk_before_it_tries_telegram() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let env = Env::new();
    // No chat id, so Telegram is never attempted and the file is the whole
    // record — which is the point of writing it first.
    let ctx = AlertContext::new(None, "main".into(), Some("j1".into()));
    news::alert::alert_failure(&ctx, "all_feeds_empty", "every feed returned 0 items");

    let log = std::fs::read_to_string(env.home.join(".nullclaw/news-failures.log")).unwrap();
    assert!(log.contains("reason: all_feeds_empty"), "{log}");
    assert!(log.contains("account: main"), "{log}");
    assert!(log.contains("job_id: j1"), "{log}");
    assert!(log.contains("deliver_to: (none)"), "{log}");
}

#[test]
fn a_repeat_alert_carries_how_often_it_has_already_fired() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let env = Env::new();
    let ctx = AlertContext::new(None, "main".into(), Some("j1".into()));
    for _ in 0..3 {
        news::alert::alert_failure(&ctx, "section_fallback_used", "tech fell back");
    }
    let log = std::fs::read_to_string(env.home.join(".nullclaw/news-failures.log")).unwrap();
    // A chronic fault should be visible in the alert itself, not only by
    // someone thinking to count the log.
    assert!(log.contains("此告警近30天已出現 1 次"), "{log}");
    assert!(log.contains("此告警近30天已出現 2 次"), "{log}");
    assert_eq!(
        news::alert::recent_failure_count("section_fallback_used", "main", 30),
        3
    );
    // A different account has its own count.
    assert_eq!(news::alert::recent_failure_count("section_fallback_used", "nunu", 30), 0);
}

// ── the theme pass ───────────────────────────────────────────────────────────

fn themed_lines() -> Vec<String> {
    (1..=4).map(|i| format!("- 第{i}則 [🔗](https://n/{i})")).collect()
}

#[test]
fn theming_off_never_calls_the_classifier() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let env = Env::new();
    std::env::set_var("NEWS_AI_THEME", "off");
    let (out, applied) = news::theme::theme_ai_section(&themed_lines(), DATE, &[]);
    assert!(!applied);
    assert_eq!(out, themed_lines());
    assert_eq!(env.calls(), 0);
}

#[test]
fn an_unrecognised_mode_is_treated_as_off() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let env = Env::new();
    std::env::set_var("NEWS_AI_THEME", "renderr");
    let (_, applied) = news::theme::theme_ai_section(&themed_lines(), DATE, &[]);
    std::env::set_var("NEWS_AI_THEME", "off");
    assert!(!applied, "a typo must not silently enable a layout change");
    assert_eq!(env.calls(), 0);
}

#[test]
fn shadow_mode_classifies_but_still_delivers_the_flat_list() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let env = Env::new();
    std::env::set_var("NEWS_AI_THEME", "shadow");
    env.reply_default(
        r#"{"labels":[{"id":1,"theme":"產品發布"},{"id":2,"theme":"產品發布"},{"id":3,"theme":"政策監管"},{"id":4,"theme":"政策監管"}]}"#,
    );
    let (out, applied) = news::theme::theme_ai_section(&themed_lines(), DATE, &[]);
    std::env::set_var("NEWS_AI_THEME", "off");

    assert_eq!(env.calls(), 1, "shadow mode still measures");
    assert!(!applied);
    assert_eq!(out, themed_lines(), "shadow must not change what ships");
    let t = env.first("ai_theme").unwrap();
    assert_eq!(t["mode"], "shadow");
    assert_eq!(t["ok"], true);
}

#[test]
fn render_mode_groups_the_lines_under_headings() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let env = Env::new();
    std::env::set_var("NEWS_AI_THEME", "render");
    env.reply_default(
        r#"{"labels":[{"id":1,"theme":"產品發布"},{"id":2,"theme":"產品發布"},{"id":3,"theme":"政策監管"},{"id":4,"theme":"政策監管"}]}"#,
    );
    let (out, applied) = news::theme::theme_ai_section(&themed_lines(), DATE, &[]);
    std::env::set_var("NEWS_AI_THEME", "off");

    assert!(applied);
    assert_eq!(out[0], "▸ 產品發布");
    assert_eq!(out[3], "▸ 政策監管");
    // Nothing is dropped — this pass only reorders.
    for i in 1..=4 {
        assert!(out.iter().any(|l| l.contains(&format!("第{i}則"))), "lost {i}: {out:?}");
    }
}

#[test]
fn a_classifier_that_answers_rubbish_leaves_the_list_flat() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let env = Env::new();
    std::env::set_var("NEWS_AI_THEME", "render");
    env.reply_default("I think these are all about products.");
    let (out, applied) = news::theme::theme_ai_section(&themed_lines(), DATE, &[]);
    std::env::set_var("NEWS_AI_THEME", "off");

    assert!(!applied);
    assert_eq!(out, themed_lines());
    assert_eq!(env.first("ai_theme").unwrap()["error"], "invalid_labels");
}

#[test]
fn theming_is_skipped_when_the_budget_cannot_also_cover_delivery() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let env = Env::new();
    std::env::set_var("NEWS_AI_THEME", "render");
    // The classifier needs ten seconds and delivery reserves thirty-four, so a
    // forty-second budget that has already run is not enough.
    std::env::set_var("NULLCLAW_SKILL_TIMEOUT", "40");
    std::env::set_var(
        "NULLCLAW_SKILL_STARTED",
        format!("{}", claw_core::budget::monotonic_secs() - 30.0),
    );
    let (_, applied) = news::theme::theme_ai_section(&themed_lines(), DATE, &[]);
    std::env::remove_var("NULLCLAW_SKILL_TIMEOUT");
    std::env::remove_var("NULLCLAW_SKILL_STARTED");
    std::env::set_var("NEWS_AI_THEME", "off");

    assert!(!applied);
    assert_eq!(env.calls(), 0, "the classifier ran with no room for delivery");
    assert_eq!(env.first("ai_theme").unwrap()["skipped"], "budget");
}

#[test]
fn a_manual_run_with_no_budget_may_always_theme() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let env = Env::new();
    std::env::set_var("NEWS_AI_THEME", "shadow");
    std::env::remove_var("NULLCLAW_SKILL_TIMEOUT");
    env.reply_default(r#"{"labels":[{"id":1,"theme":"產品發布"},{"id":2,"theme":"產品發布"},{"id":3,"theme":"其他"},{"id":4,"theme":"其他"}]}"#);
    news::theme::theme_ai_section(&themed_lines(), DATE, &[]);
    std::env::set_var("NEWS_AI_THEME", "off");
    assert_eq!(env.calls(), 1);
}

#[test]
fn a_placeholder_section_is_never_themed() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let env = Env::new();
    std::env::set_var("NEWS_AI_THEME", "render");
    let flat = vec!["- 今日無相關新聞".to_string()];
    let (out, applied) = news::theme::theme_ai_section(&flat, DATE, &[]);
    std::env::set_var("NEWS_AI_THEME", "off");
    assert!(!applied);
    assert_eq!(out, flat);
    assert_eq!(env.calls(), 0);
    assert_eq!(env.first("ai_theme").unwrap()["skipped"], "placeholder");
}
