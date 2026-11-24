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
use news::config::{AI_SUBSTAGE_CACHE_VARIANT, DEDUP_RULES, TRANSLATION_RULES_STRICT};
use news::precheck::new_cache;
use news::render::LinkMap;
use news::select::NumberedMap;
use news::summarize::{
    run_ai_substage, run_custom_topic, summarize_default_ai_substaged, summarize_default_section,
};
use news::text::Item;
use serde_json::{json, Value};
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
fn a_pick_that_decodes_to_a_podcast_page_is_dropped_and_traced_as_media() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let env = Env::new();
    // Seed the decode cache — the file `decode_google_news_url` consults before
    // any network — so item 2 resolves to a podcast page while staying offline.
    // The page is on the unroutable host too: if the path rule were gone, the
    // fetch would fail and the item would be kept as `fetch_error`.
    let link = "http://127.0.0.1:1/2";
    use sha1::Digest;
    let hex: String = sha1::Sha1::digest(link.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let day = env
        .home
        .join(".nullclaw/.news-quality-cache")
        .join(jiff::Zoned::now().date().to_string());
    std::fs::create_dir_all(&day).unwrap();
    std::fs::write(
        day.join(format!("{hex}.url")),
        "http://127.0.0.1:1/podcast/news/5/memory-bear-market/2722624",
    )
    .unwrap();

    env.reply_default("- #1 美半導體危機影響台積電美光三星\n- #2 記憶體股技術性熊市\n- #3 半導體股震盪何時完結");
    let (lines, _) = summarize_default_section(
        "tech",
        &items(&TECH_FIXTURE),
        DATE,
        &LinkMap::default(),
        &new_cache(),
    );

    let t2 = env.first("quality_tier2").expect("precheck ran");
    assert_eq!(t2["dropped"], 1, "{t2}");
    assert_eq!(t2["drop_reasons"]["media"], 1, "{t2}");
    let body = lines.join("\n");
    assert!(!body.contains("記憶體股技術性熊市"), "media pick delivered: {body}");
    assert!(body.contains("美半導體危機影響台積電美光三星"), "{body}");
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
fn a_reply_that_repeats_its_picks_ships_each_story_once() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    const REPEATED: &str = "- #2 行政院通過明年度總預算案\n- #6 法國總統抵北京談氣候\n\
- #2 行政院通過明年度總預算案\n- #6 法國總統抵北京談氣候";

    let env = Env::new();
    env.reply_default(REPEATED);
    assert_eq!(general(), (good_general_lines(), false));
    assert_eq!(
        env.all("llm_repeated_marker_dropped"),
        vec![json!({"section": "general", "repeated": [2, 6]})]
    );
    drop(env);

    let env = Env::new();
    env.reply_default(REPEATED);
    let topic = run_custom_topic("節稅", &items(&GENERAL_FIXTURE), DATE, &new_cache());
    assert_eq!(topic.unwrap(), good_general_lines());
    drop(env);

    let env = Env::new();
    env.reply_default("- #1 日本加碼算力補助\n- #2 工研院開源醫療影像系統\n- #1 日本加碼算力補助");
    let lines = run_ai_substage(&items(&AI_FIXTURE), 2, 4, DATE, &new_cache()).unwrap();
    assert_eq!(lines, vec![line("日本加碼算力補助", 3), line("工研院開源醫療影像系統", 4)]);
    drop(env);

    // The kill switch is for the similarity collapse, not for this.
    let env = Env::new();
    std::env::set_var("NEWS_LLM_POST_DEDUP", "0");
    env.reply_default(REPEATED);
    let got = general();
    std::env::remove_var("NEWS_LLM_POST_DEDUP");
    assert_eq!(env.all("llm_post_dedup")[0]["enabled"], false);
    assert_eq!(got, (good_general_lines(), false));
}

#[test]
fn a_translation_that_repeats_its_headlines_ships_each_story_once() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let env = Env::new();
    env.reply(1, "- #2 Cabinet approves the budget\n- #6 French president visits Beijing").reply(
        2,
        "- #2 行政院通過明年度總預算案\n- #6 法國總統抵北京談氣候\n- #2 行政院通過預算",
    );
    assert_eq!(general(), (good_general_lines(), false));
    assert_eq!(
        env.all("llm_repeated_marker_dropped"),
        vec![json!({"section": "general", "repeated": [2]})]
    );
}

#[test]
fn a_translation_is_language_checked_on_what_it_would_ship() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let env = Env::new();
    // Five bullets, four of them Chinese; the two that remain are half English.
    env.reply(1, "- #2 Cabinet approves the budget\n- #6 French president visits Beijing").reply(
        2,
        "- #2 Cabinet approves the budget\n- #6 法國總統抵北京談氣候\n\
- #2 行政院通過預算\n- #2 行政院通過總預算\n- #6 法國總統訪北京",
    );
    let (lines, used_fallback) = general();
    assert!(used_fallback, "{lines:?}");
    let rejected = env.all("llm_validation_failed");
    assert_eq!(rejected.last().unwrap()["reason"], "translation_retry_validation");
}

#[test]
fn a_reply_without_repeats_traces_none() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let env = Env::new();
    env.reply_default(GOOD_GENERAL);
    assert_eq!(general(), (good_general_lines(), false));
    assert_eq!(env.all("llm_repeated_marker_dropped"), Vec::<Value>::new());
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

#[test]
fn a_custom_topic_that_answers_the_placeholder_is_not_a_failure() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let env = Env::new();
    // The regression: the prompt tells the model to answer exactly this when
    // nothing is worth reporting, and the marker gate then counted zero
    // bullets and called it a protocol violation. The caller turns an Err into
    // a raw listing of every candidate plus a Telegram alert — publishing the
    // items the model just rejected. 18 such `marked=0/0` events sit in the
    // trace since 2026-05-11; the six since 2026-08-07 are the ones verified
    // as single-candidate sentinel replies.
    env.reply_default("- 今日無相關新聞");
    let out = run_custom_topic("頂端客戶群", &items(&TECH_FIXTURE[..1]), DATE, &new_cache())
        .expect("the placeholder is an answer, not a failure");
    assert_eq!(out, vec!["- 今日無相關新聞"]);
    assert!(
        env.first("custom_topic_no_news").is_some(),
        "the deliberate no-news answer should be visible in the trace"
    );
}

#[test]
fn a_custom_topic_placeholder_is_accepted_however_the_model_dresses_it() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    for reply in ["今日無相關新聞", "「今日無相關新聞」", "  - 今日無相關新聞  \n\n"] {
        let env = Env::new();
        env.reply_default(reply);
        let out = run_custom_topic("頂端客戶群", &items(&TECH_FIXTURE[..1]), DATE, &new_cache())
            .unwrap_or_else(|e| panic!("{reply:?} rejected: {e}"));
        assert_eq!(out, vec!["- 今日無相關新聞"]);
    }
}

#[test]
fn a_custom_topic_placeholder_buried_in_prose_still_falls_back() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let env = Env::new();
    // The narrow gate earns its keep here: a model that explains itself has
    // not answered the question, so the raw-listing fallback must still fire.
    env.reply_default("我看過了，這則新聞太瑣碎。\n- 今日無相關新聞");
    let err = run_custom_topic("頂端客戶群", &items(&TECH_FIXTURE[..1]), DATE, &new_cache())
        .expect_err("prose plus the placeholder is not an answer");
    assert!(err.starts_with("marker_validation"), "{err}");
}

#[test]
fn a_rejected_custom_topic_reply_is_recorded_verbatim() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let env = Env::new();
    // Until this landed the trace kept only `stdout_len`, so diagnosing a
    // fallback meant guessing the reply from its character count.
    env.reply_default("這批新聞我覺得都不重要");
    let _ = run_custom_topic("頂端客戶群", &items(&TECH_FIXTURE[..1]), DATE, &new_cache())
        .expect_err("unmarked prose is a fallback");
    let logged = env
        .first("llm_validation_failed")
        .expect("the rejected reply must reach the trace");
    assert_eq!(logged["topic"], "頂端客戶群");
    assert!(
        logged["stdout_sample"]
            .as_str()
            .unwrap_or_default()
            .contains("我覺得都不重要"),
        "{logged}"
    );
}

// ── the reformat retry: a rewritten prompt for a wrong-shaped reply ──────────

/// Eight unrelated headlines, so nothing collapses and nothing is refilled.
const GENERAL_FIXTURE: [&str; 8] = [
    "氣象署：北部午後雷陣雨 高溫上看32度 - 中央社",
    "行政院拍板明年度總預算 歲出破三兆元 - 公視新聞",
    "聯準會維持利率不變 鮑爾示警通膨 - 經濟日報",
    "花蓮外海規模5.8地震 無人傷亡 - 自由時報",
    "英國首相宣布提前大選 - BBC中文",
    "法國總統訪問北京 會談聚焦氣候 - 法新社",
    "東京奧運場館改建案流標 - 日經中文網",
    "高鐵票價明年春節前調漲 - 聯合報",
];

/// 2026-11-23 08:30, `general`: legal `- #N` bullets, some twice, around
/// unbulleted chain of thought. Every bullet is marked, so only the shape
/// gate rejects it.
const REPLY_1123: &str = "- #2 行政院拍板明年度總預算\n\
- #6 法國總統訪問北京\n\
Wait — #1 is a weather forecast, not major news.\n\
Let me pick #2, #6, #1.\n\
- #2 行政院拍板明年度總預算\n\
- #6 法國總統訪問北京\n\
- #1 北部午後雷陣雨";

const GOOD_GENERAL: &str = "- #2 行政院通過明年度總預算案\n- #6 法國總統抵北京談氣候";

/// Four AI candidates: halves [0,2) and [2,4).
const AI_FIXTURE: [&str; 4] = [
    "OpenAI 發表新一代推理模型 - iThome",
    "歐盟通過人工智慧法案修正 - 聯合報",
    "日本擴大算力補助計畫 - 日經中文網",
    "工研院開源醫療影像辨識系統 - 中央社",
];

const COT_AI: &str = "- #1 OpenAI 發表新模型\n讓我再想想，#2 比較重要。\n- #2 歐盟修正人工智慧法案";

fn general() -> (Vec<String>, bool) {
    summarize_default_section(
        "general",
        &items(&GENERAL_FIXTURE),
        DATE,
        &LinkMap::default(),
        &new_cache(),
    )
}

fn ai_section() -> Result<Vec<String>, String> {
    let ctx = AlertContext::new(None, "main".into(), None);
    summarize_default_ai_substaged(&items(&AI_FIXTURE), DATE, &ctx, &new_cache())
        .map_err(|e| e.0)
}

fn line(title: &str, n: usize) -> String {
    format!("- {title} [🔗](http://127.0.0.1:1/{n})")
}

fn general_fallback() -> Vec<String> {
    (0..3).map(|i| line(GENERAL_FIXTURE[i], i + 1)).collect()
}

fn good_general_lines() -> Vec<String> {
    vec![line("行政院通過明年度總預算案", 2), line("法國總統抵北京談氣候", 6)]
}

/// An event without the envelope fields every trace line carries.
fn payload(mut v: Value) -> Value {
    for k in ["ts", "job_id", "skill", "event"] {
        assert!(v.as_object_mut().unwrap().remove(k).is_some(), "{k} missing");
    }
    v
}

impl Env {
    fn all(&self, event: &str) -> Vec<Value> {
        self.traces()
            .into_iter()
            .filter(|e| e["event"] == event)
            .map(payload)
            .collect()
    }

    /// `(variant, timeout)` of every model call, in order.
    fn asked(&self) -> Vec<(String, u64)> {
        self.all("llm_agent_start")
            .iter()
            .map(|e| {
                (
                    e["variant"].as_str().unwrap().to_string(),
                    e["timeout_secs"].as_u64().unwrap(),
                )
            })
            .collect()
    }

    /// `(variant, reason, range, sample)` of every rejected reply, in order.
    fn rejected(&self) -> Vec<(String, String, Value, String)> {
        self.all("llm_validation_failed")
            .iter()
            .map(|e| {
                (
                    e["variant"].as_str().unwrap().to_string(),
                    e["reason"].as_str().unwrap().to_string(),
                    e["range"].clone(),
                    e["stdout_sample"].as_str().unwrap().to_string(),
                )
            })
            .collect()
    }

    fn no_reformat_retry(&self) {
        for event in [
            "llm_agent_reformat_retry",
            "section_reformat_retry_ok",
            "section_reformat_retry_failed",
            "custom_topic_reformat_retry_ok",
            "custom_topic_reformat_retry_failed",
        ] {
            assert_eq!(self.all(event), Vec::<Value>::new(), "{event}");
        }
    }
}

fn ai_variant(start: usize, end: usize) -> String {
    format!("{AI_SUBSTAGE_CACHE_VARIANT}_{start}_{end}")
}

#[test]
fn a_general_reply_wrapped_in_reasoning_is_rescued_by_one_reformat_retry() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let env = Env::new();
    env.reply(1, REPLY_1123).reply(2, GOOD_GENERAL);
    let (lines, used_fallback) = general();

    assert_eq!(lines, good_general_lines());
    assert!(!used_fallback);
    assert_eq!(env.calls(), 2);
    assert_eq!(
        env.asked(),
        vec![
            ("default_general".to_string(), 90),
            ("default_general_reformat".to_string(), 60),
        ]
    );
    assert_eq!(
        env.all("section_reformat_retry_ok"),
        vec![json!({"section": "general", "attempt": 2, "first_sample": REPLY_1123})]
    );
    assert_eq!(env.all("section_reformat_retry_failed"), Vec::<Value>::new());
    // A rescued reply is not a failure, and the picks are the reply that shipped.
    assert_eq!(env.rejected(), vec![]);
    let picks = env.all("section_picks");
    assert_eq!(picks.len(), 1, "{picks:?}");
    assert_eq!(
        picks[0]["picks"],
        json!([
            {"n": 2, "source": GENERAL_FIXTURE[1], "written": "行政院通過明年度總預算案"},
            {"n": 6, "source": GENERAL_FIXTURE[5], "written": "法國總統抵北京談氣候"},
        ])
    );
}

#[test]
fn the_rejected_first_reply_is_clipped_to_1200_characters_in_the_ok_event() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let env = Env::new();
    let long = format!("- #2 行政院\n{}", "想".repeat(2000));
    env.reply(1, &long).reply(2, GOOD_GENERAL);
    general();
    let clipped: String = long.chars().take(1200).collect();
    assert_eq!(env.all("section_reformat_retry_ok")[0]["first_sample"], clipped);
}

#[test]
fn the_retry_prompt_restates_the_task_without_asking_for_verbatim_titles() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let env = Env::new();
    env.reply(1, REPLY_1123).reply(2, GOOD_GENERAL);
    general();
    let p = env.prompt(2);
    for (i, title) in GENERAL_FIXTURE.iter().enumerate() {
        assert!(p.contains(&format!("#{} {title}", i + 1)), "candidate {}:\n{p}", i + 1);
    }
    for part in [
        "不合格",
        "分析與評論",
        "2-3 則最重大的非科技一般新聞",
        "- #N 新聞標題",
        TRANSLATION_RULES_STRICT.as_str(),
        "排除瑣碎的、純行銷推廣的、政治宣傳性質的、投資建議類新聞",
        "同一事件若有多則候選，優先挑選標題明確點出關鍵實體",
        DEDUP_RULES,
        "每行必須以繁體中文新聞句子開始",
    ] {
        assert!(p.contains(part), "missing {part:?}:\n{p}");
    }
    // The custom topic's rule contradicts translating an English candidate.
    assert!(!p.contains("原樣照抄"), "{p}");
    assert!(!env.prompt(1).contains("不合格"), "the first prompt changed");
}

#[test]
fn a_bad_retry_falls_back_with_the_original_reply_logged() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // Each second reply fails exactly one acceptance condition.
    for (second, rc, error) in [
        ("- #6 法國總統訪問北京\n我選這一則。", 0, "second validation failed"),
        ("- #2 行政院通過預算\n- 沒有編號", 0, "second validation failed"),
        ("- #2 行政院通過預算\n- #99 不在候選內", 0, "second validation failed"),
        ("我需要更多資訊", 0, "second validation failed"),
        ("", 0, "second validation failed"),
        ("- 今日無相關新聞", 0, "second validation failed"),
        // Well-formed text from a call that did not succeed.
        (GOOD_GENERAL, 1, "second validation failed"),
        (GOOD_GENERAL, 124, "timeout"),
    ] {
        let env = Env::new();
        env.reply(1, REPLY_1123).reply(2, second).exit_code(2, rc);
        let (lines, used_fallback) = general();

        assert_eq!(lines, general_fallback(), "{second:?} rc={rc}");
        assert!(used_fallback, "{second:?} rc={rc}");
        assert_eq!(env.calls(), 2, "exactly one retry: {second:?} rc={rc}");
        assert_eq!(
            env.all("section_reformat_retry_failed"),
            vec![json!({"section": "general", "attempt": 2, "error": error,
                        "retry_sample": second})],
        );
        assert_eq!(env.all("section_reformat_retry_ok"), Vec::<Value>::new());
        assert_eq!(
            env.rejected(),
            vec![(
                "default_general".to_string(),
                "shape_validation".to_string(),
                Value::Null,
                REPLY_1123.to_string(),
            )]
        );
        assert_eq!(env.all("section_picks"), Vec::<Value>::new());
    }
}

#[test]
fn a_good_first_reply_makes_no_extra_call() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let env = Env::new();
    env.reply_default(GOOD_GENERAL);
    assert_eq!(general(), (good_general_lines(), false));
    assert_eq!(env.asked(), vec![("default_general".to_string(), 90)]);
    env.no_reformat_retry();

    let env = Env::new();
    env.reply(1, "- #1 OpenAI 發表新模型\n- #2 歐盟修正人工智慧法案")
        .reply(2, "- #1 日本加碼算力補助\n- #2 工研院開源醫療影像系統");
    ai_section().unwrap();
    assert_eq!(env.asked(), vec![(ai_variant(0, 2), 60), (ai_variant(2, 4), 60)]);
    env.no_reformat_retry();
}

#[test]
fn only_a_marked_reply_of_the_wrong_shape_is_retried() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    for (reply, rc, calls) in [
        // marker_validation: an unmarked bullet, an unknown id, no bullet.
        ("- #2 行政院通過預算\n- 沒有編號\n想一想", 0, 1),
        ("- #2 行政院通過預算\n- #99 不在候選內\n想一想", 0, 1),
        ("我需要更多資訊", 0, 1),
        ("", 0, 1),
        // The wrong shape, but from a call that failed.
        (REPLY_1123, 1, 1),
        // A stall already spent its own same-prompt retry.
        (REPLY_1123, 124, 2),
    ] {
        let env = Env::new();
        env.reply_default(reply).exit_code(1, rc).exit_code(2, rc);
        let (lines, used_fallback) = general();
        assert_eq!(lines, general_fallback(), "{reply:?} rc={rc}");
        assert!(used_fallback);
        assert_eq!(env.calls(), calls, "{reply:?} rc={rc}");
        env.no_reformat_retry();
    }

    let env = Env::new();
    env.reply_default("- #1 OpenAI 發表新模型\n- 沒有編號\n想一想");
    let err = run_ai_substage(&items(&AI_FIXTURE), 0, 2, DATE, &new_cache()).unwrap_err();
    assert_eq!(err, "marker_validation marked=1/2");
    assert_eq!(env.calls(), 1);
    env.no_reformat_retry();
}

#[test]
fn the_reformat_retry_is_skipped_when_the_cron_budget_cannot_fit_it() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let run = |elapsed: f64| {
        let env = Env::new();
        env.reply(1, REPLY_1123).reply(2, GOOD_GENERAL);
        std::env::set_var("NULLCLAW_SKILL_TIMEOUT", "100");
        std::env::set_var(
            "NULLCLAW_SKILL_STARTED",
            format!("{}", claw_core::budget::monotonic_secs() - elapsed),
        );
        let got = general();
        std::env::remove_var("NULLCLAW_SKILL_TIMEOUT");
        std::env::remove_var("NULLCLAW_SKILL_STARTED");
        (env, got)
    };

    // 100 - 50 - 2 reserved = 48s, short of the retry's 60.
    let (env, got) = run(50.0);
    assert_eq!(got, (general_fallback(), true));
    assert_eq!(env.calls(), 1);
    env.no_reformat_retry();
    let skipped = env.all("llm_agent_reformat_skipped_budget");
    assert_eq!(skipped.len(), 1, "{skipped:?}");
    assert_eq!(skipped[0]["variant"], "default_general_reformat");
    assert_eq!(skipped[0]["retry_timeout"], 60);
    drop(env);

    // 78s fits it.
    let (env, got) = run(20.0);
    assert_eq!(got, (good_general_lines(), false));
    assert_eq!(env.calls(), 2);
    assert_eq!(env.all("llm_agent_reformat_skipped_budget"), Vec::<Value>::new());
}

#[test]
fn after_an_accepted_retry_every_later_trace_describes_the_accepted_reply() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // Five marked bullets rejected, two accepted: the counts tell them apart.
    const ENGLISH: &str = "- #2 Cabinet approves the budget\n- #6 French president visits Beijing";
    let env = Env::new();
    env.reply(1, REPLY_1123)
        .reply(2, ENGLISH)
        .reply(3, "- #2 行政院通過明年度總預算案\n- #6 法國總統抵北京談氣候");
    assert_eq!(general(), (good_general_lines(), false));
    assert_eq!(env.calls(), 3, "reply, reformat, translation");
    let failed = env.all("llm_validation_failed");
    assert_eq!(failed.len(), 1, "{failed:?}");
    assert_eq!(failed[0]["reason"], "language_validation");
    assert_eq!(failed[0]["variant"], "default_general");
    assert_eq!(failed[0]["stdout_len"], ENGLISH.chars().count());
    assert_eq!(failed[0]["stdout_sample"], ENGLISH);
    assert_eq!(failed[0]["marked_bullets"], 2);
    assert_eq!(failed[0]["total_bullets"], 2);
    drop(env);

    let env = Env::new();
    std::env::set_var("NEWS_PRECHECK", "0");
    env.reply(1, REPLY_1123).reply(2, GOOD_GENERAL);
    let mut linkless = items(&GENERAL_FIXTURE);
    for i in &mut linkless {
        i.link = String::new();
    }
    let (_, used_fallback) = summarize_default_section(
        "general",
        &linkless,
        DATE,
        &LinkMap::default(),
        &new_cache(),
    );
    assert!(used_fallback);
    let failed = env.all("llm_link_validation_failed");
    assert_eq!(failed.len(), 1, "{failed:?}");
    assert_eq!(failed[0]["stdout_len"], GOOD_GENERAL.chars().count());
    assert_eq!(failed[0]["stdout_sample"], GOOD_GENERAL);
    assert_eq!(failed[0]["marked_bullets"], 2);
    assert_eq!(failed[0]["total_bullets"], 2);
}

#[test]
fn an_ai_half_rescued_by_the_retry_is_never_split() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let env = Env::new();
    env.reply(1, COT_AI)
        .reply(2, "- #1 OpenAI 推出推理模型\n- #2 歐盟敲定人工智慧法案")
        .reply(3, "- #1 日本加碼算力補助\n- #2 工研院開源醫療影像系統");
    let expected = vec![
        line("OpenAI 推出推理模型", 1),
        line("歐盟敲定人工智慧法案", 2),
        line("日本加碼算力補助", 3),
        line("工研院開源醫療影像系統", 4),
    ];
    assert_eq!(ai_section().unwrap(), expected);
    assert_eq!(
        env.asked(),
        vec![
            (ai_variant(0, 2), 60),
            (format!("{}_reformat", ai_variant(0, 2)), 60),
            (ai_variant(2, 4), 60),
        ]
    );
    assert_eq!(env.calls(), 3);
    assert_eq!(
        env.all("section_reformat_retry_ok"),
        vec![json!({"section": "ai", "range": [0, 2], "attempt": 2, "first_sample": COT_AI})]
    );
    assert_eq!(env.all("ai_substage_level2_failed"), Vec::<Value>::new());
    assert_eq!(env.all("ai_substage_level3_start"), Vec::<Value>::new());
    assert_eq!(env.rejected(), vec![]);

    let p = env.prompt(2);
    for part in [
        "#1 OpenAI 發表新一代推理模型 - iThome",
        "#2 歐盟通過人工智慧法案修正 - 聯合報",
        "2 則重大研究突破、政策變化、產品發布、產業併購、國安與監管等真正有影響力的 AI 新聞",
        TRANSLATION_RULES_STRICT.as_str(),
        "排除瑣碎的、純行銷推廣的、政治宣傳性質的、投資建議類新聞",
        "同一事件若有多則候選，優先挑選標題明確點出關鍵實體",
        DEDUP_RULES,
    ] {
        assert!(p.contains(part), "missing {part:?}:\n{p}");
    }
    assert!(!p.contains("原樣照抄"), "{p}");
    // The other half's candidates are not offered.
    assert!(!p.contains(AI_FIXTURE[2]), "{p}");

    // The rescued reply is what the day cache holds, under the usual key.
    let calls = env.calls();
    assert_eq!(ai_section().unwrap(), expected);
    assert_eq!(env.calls(), calls, "the rescued half was re-asked");
}

#[test]
fn an_ai_half_whose_retry_also_fails_is_split_into_quarters_with_retries_of_their_own() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let env = Env::new();
    const SECOND: &str = "- #2 歐盟修正人工智慧法案\n這是我的選擇。";
    const COT_QUARTER: &str = "- #1 歐盟修正\n等等，再看一次。";
    env.reply(1, COT_AI)
        .reply(2, SECOND)
        .reply(3, "- #1 日本加碼算力補助\n- #2 工研院開源醫療影像系統")
        .reply(4, "- #1 OpenAI 推出推理模型")
        .reply(5, COT_QUARTER)
        .reply(6, "- #1 歐盟敲定人工智慧法案");
    assert_eq!(
        ai_section().unwrap(),
        vec![
            line("OpenAI 推出推理模型", 1),
            line("歐盟敲定人工智慧法案", 2),
            line("日本加碼算力補助", 3),
            line("工研院開源醫療影像系統", 4),
        ]
    );
    assert_eq!(env.calls(), 6);
    assert_eq!(
        env.asked().into_iter().map(|(v, _)| v).collect::<Vec<_>>(),
        vec![
            ai_variant(0, 2),
            format!("{}_reformat", ai_variant(0, 2)),
            ai_variant(2, 4),
            ai_variant(0, 1),
            ai_variant(1, 2),
            format!("{}_reformat", ai_variant(1, 2)),
        ]
    );
    assert_eq!(
        env.all("section_reformat_retry_failed"),
        vec![json!({"section": "ai", "range": [0, 2], "attempt": 2,
                    "error": "second validation failed", "retry_sample": SECOND})]
    );
    assert_eq!(
        env.all("section_reformat_retry_ok"),
        vec![json!({"section": "ai", "range": [1, 2], "attempt": 2,
                    "first_sample": COT_QUARTER})]
    );
    assert_eq!(
        env.all("ai_substage_level2_failed"),
        vec![json!({"half": 0, "range": [0, 2], "error": "shape_validation"})]
    );
    let failed = env.all("llm_validation_failed");
    assert_eq!(failed.len(), 1, "{failed:?}");
    assert_eq!(failed[0]["section"], "ai");
    assert_eq!(
        env.rejected(),
        vec![(
            ai_variant(0, 2),
            "shape_validation".to_string(),
            json!([0, 2]),
            COT_AI.to_string(),
        )]
    );
}

#[test]
fn every_ai_reply_that_is_rejected_reaches_the_trace_with_its_range() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    const ENGLISH: &str = "- #1 Japan expands compute subsidies\n- #2 ITRI opens imaging system";
    for (first, second, linkless, err, reason, sample) in [
        ("", "", false, "empty_stdout", "empty_stdout", ""),
        (
            "我需要更多資訊",
            "",
            false,
            "marker_validation marked=0/0",
            "marker_validation",
            "我需要更多資訊",
        ),
        (
            "- #1 日本補助\n想一想",
            "還是想",
            false,
            "shape_validation",
            "shape_validation",
            "- #1 日本補助\n想一想",
        ),
        (ENGLISH, "沒有編號", false, "language_validation", "language_validation", ENGLISH),
        (
            "- #1 日本加碼算力補助\n- #2 工研院開源醫療影像系統",
            "",
            true,
            "no_links_attached",
            "no_links_attached",
            "- #1 日本加碼算力補助\n- #2 工研院開源醫療影像系統",
        ),
    ] {
        let env = Env::new();
        env.reply(1, first).reply(2, second);
        let mut batch = items(&AI_FIXTURE);
        if linkless {
            std::env::set_var("NEWS_PRECHECK", "0");
            for i in &mut batch {
                i.link = String::new();
            }
        }
        // The second half, so the range cannot be mistaken for a default.
        let got = run_ai_substage(&batch, 2, 4, DATE, &new_cache());
        assert_eq!(got.unwrap_err(), err);
        let events: Vec<Value> = env
            .all("llm_validation_failed")
            .into_iter()
            .filter(|e| e["reason"] == reason)
            .collect();
        assert_eq!(events.len(), 1, "{reason}: {events:?}");
        assert_eq!(events[0]["variant"], ai_variant(2, 4), "{reason}");
        assert_eq!(events[0]["section"], "ai", "{reason}");
        assert_eq!(events[0]["range"], json!([2, 4]), "{reason}");
        assert_eq!(events[0]["stdout_sample"], sample, "{reason}");
        assert_eq!(events[0]["stdout_len"], first.chars().count(), "{reason}");
        assert_eq!(events[0]["items_numbered"], 2, "{reason}");
    }

    // A call that failed returned no reply to reject.
    let env = Env::new();
    env.reply_default("- #1 日本補助").exit_code(1, 7);
    let err = run_ai_substage(&items(&AI_FIXTURE), 2, 4, DATE, &new_cache()).unwrap_err();
    assert_eq!(err, "exit_code=7");
    assert_eq!(env.rejected(), vec![]);
}

#[test]
fn an_ai_retry_accepted_in_english_is_rejected_as_the_accepted_reply() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let env = Env::new();
    const ENGLISH: &str = "- #1 Japan expands subsidies";
    env.reply(1, "- #1 日本補助\n- #2 工研院\n想一想")
        .reply(2, ENGLISH)
        .reply(3, "沒有編號");
    let err = run_ai_substage(&items(&AI_FIXTURE), 2, 4, DATE, &new_cache()).unwrap_err();
    assert_eq!(err, "language_validation");
    let events = env.all("llm_validation_failed");
    let e = events
        .iter()
        .find(|e| e["reason"] == "language_validation")
        .expect("traced");
    assert_eq!(e["stdout_len"], ENGLISH.chars().count());
    assert_eq!(e["stdout_sample"], ENGLISH);
    assert_eq!(e["marked_bullets"], 1);
    assert_eq!(e["total_bullets"], 1);
}

#[test]
fn a_custom_topic_reply_of_the_wrong_shape_is_rescued_and_cached() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let env = Env::new();
    env.reply(1, REPLY_1123).reply(2, GOOD_GENERAL);
    let topic = || run_custom_topic("富人", &items(&GENERAL_FIXTURE), DATE, &new_cache());
    assert_eq!(topic().unwrap(), good_general_lines());
    assert_eq!(
        env.asked(),
        vec![
            ("custom_topic_v5_author_deny_富人".to_string(), 60),
            ("custom_topic_v5_author_deny_富人_reformat".to_string(), 60),
        ]
    );
    assert_eq!(
        env.all("custom_topic_reformat_retry_ok"),
        vec![json!({"topic": "富人", "attempt": 2, "first_sample": REPLY_1123})]
    );
    assert_eq!(env.rejected(), vec![]);
    // Its own prompt, not the sections'.
    assert!(env.prompt(2).starts_with("你是新聞編輯。你上一次針對「富人」的輸出不合格"));
    assert!(env.prompt(2).contains("標題必須原樣照抄候選標題"));

    assert_eq!(topic().unwrap(), good_general_lines());
    assert_eq!(env.calls(), 2, "the rescued topic was not cached");
}

#[test]
fn a_custom_topic_whose_retry_also_fails_reports_the_original_reply() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    for (second, rc, error) in [
        ("- #6 法國總統訪問北京\n我選這一則。", 0, "second validation failed"),
        ("- #2 行政院通過預算\n- 沒有編號", 0, "second validation failed"),
        (GOOD_GENERAL, 1, "second validation failed"),
        (GOOD_GENERAL, 124, "timeout"),
    ] {
        let env = Env::new();
        env.reply(1, REPLY_1123).reply(2, second).exit_code(2, rc);
        let err = run_custom_topic("富人", &items(&GENERAL_FIXTURE), DATE, &new_cache())
            .unwrap_err();
        assert_eq!(err, "shape_validation");
        assert_eq!(env.calls(), 2);
        assert_eq!(
            env.all("custom_topic_reformat_retry_failed"),
            vec![json!({"topic": "富人", "attempt": 2, "error": error,
                        "retry_sample": second})]
        );
        assert_eq!(
            env.rejected(),
            vec![(
                "custom_topic_v5_author_deny_富人".to_string(),
                "shape_validation".to_string(),
                Value::Null,
                REPLY_1123.to_string(),
            )]
        );
    }
}

// ── a leaked tool-call tag ───────────────────────────────────────────────────

#[test]
fn a_leading_orphan_tool_call_tag_is_stripped_and_costs_no_retry() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let env = Env::new();
    // 2026-11-24 08:30, `general`.
    let reply = "</tool_call>- #6 法國總統抵北京談氣候\n- #1 北部午後雷陣雨\n- #4 花蓮外海地震";
    env.reply_default(reply);
    let (lines, used_fallback) = general();
    assert_eq!(
        lines,
        vec![
            line("法國總統抵北京談氣候", 6),
            line("北部午後雷陣雨", 1),
            line("花蓮外海地震", 4),
        ]
    );
    assert!(!used_fallback);
    assert_eq!(env.calls(), 1);
    env.no_reformat_retry();
    assert_eq!(
        env.all("llm_agent_artifact_stripped"),
        vec![json!({"variant": "default_general", "artifact": "</tool_call>", "count": 1})]
    );
    assert_eq!(env.all("llm_agent_exit")[0]["stdout_len"], reply.chars().count());
}

#[test]
fn every_leading_tag_is_stripped_and_counted_in_one_event() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let env = Env::new();
    env.reply_default("</tool_call>\n</tool_call>\n- #1 甲\n");
    let r = run_agent("p", 5, "v", &[], &NumberedMap::new());
    assert_eq!(r.stdout, "\n- #1 甲\n");
    assert_eq!(
        env.all("llm_agent_artifact_stripped"),
        vec![json!({"variant": "v", "artifact": "</tool_call>", "count": 2})]
    );
}

#[test]
fn a_reply_without_the_tag_reaches_the_caller_byte_for_byte() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let env = Env::new();
    env.reply_default("\n  - #1 甲 </tool_call>\n\n");
    let r = run_agent("p", 5, "v", &[], &NumberedMap::new());
    assert_eq!(r.stdout, "\n  - #1 甲 </tool_call>\n\n");
    assert_eq!(env.all("llm_agent_artifact_stripped"), Vec::<Value>::new());
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
