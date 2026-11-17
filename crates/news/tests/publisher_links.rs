//! Rendering the publisher's URL instead of the Google News redirect.
//!
//! 2026-11: the operator tapped a digest link on a phone over a mobile network
//! and got Google's "Sorry... automated queries" page instead of the article.
//! Tier 2 already decodes every selected pick to its publisher URL; the
//! renderer now uses it, and keeps the Google link only where the decode did
//! not resolve.
//!
//! Offline: every verdict is pre-seeded into the shared precheck cache, keyed
//! by the item's link exactly as the real precheck keys it, so no decode or
//! fetch is attempted. The model is a stub that prints a canned reply.

use news::alert::AlertContext;
use news::digest::{summarize_llm, summarize_llm_custom};
use news::precheck::{new_cache, publisher_link, SharedCache};
use news::quality::{Action, Verdict};
use news::render::{LinkMap, PAYWALL_NOTE};
use news::summarize::{run_ai_substage, run_custom_topic, summarize_default_section};
use news::text::Item;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard};

const DATE: &str = "2026/11/17 (Tue)";

const G1: &str = "https://news.google.com/rss/articles/CBMi-one?oc=5";
const G2: &str = "https://news.google.com/rss/articles/CBMi-two?oc=5";
const G3: &str = "https://news.google.com/rss/articles/CBMi-three?oc=5";
const G4: &str = "https://news.google.com/rss/articles/CBMi-four?oc=5";

const P1: &str = "https://money.udn.com/money/story/5612/8301234";
const P3: &str = "https://radio.example.com/podcast/ai-talk-3";
const P4: &str = "https://www.wsj.com/economy/fed-rate-path-4";

// HOME is process-wide and every test here sets it, so they serialise. One
// directory per test, never per process: the section caches live under HOME
// and a shared one would replay the previous test's digest.
static ENV_LOCK: Mutex<()> = Mutex::new(());
static SEQ: AtomicUsize = AtomicUsize::new(0);

struct Env {
    home: PathBuf,
    _guard: MutexGuard<'static, ()>,
}

impl Env {
    fn new(precheck: bool) -> Env {
        let guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = std::env::temp_dir().join(format!(
            "news-publisher-links-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join(".nullclaw")).unwrap();
        std::fs::create_dir_all(home.join("stub")).unwrap();
        let bin = home.join("nullclaw/zig-out/bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(
            bin.join("nullclaw"),
            "#!/bin/sh\ncat \"$HOME/stub/resp.default\"\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(bin.join("nullclaw"), std::fs::Permissions::from_mode(0o755))
                .unwrap();
        }
        std::fs::write(
            home.join("stub/resp.default"),
            "- #1 台積電法說會釋出樂觀展望\n\
             - #2 輝達財報優於市場預期\n\
             - #3 播客節目談人工智慧發展\n\
             - #4 華爾街分析聯準會利率路徑",
        )
        .unwrap();
        std::env::set_var("HOME", &home);
        std::env::set_var("NEWS_PRECHECK", if precheck { "1" } else { "0" });
        std::env::set_var("NEWS_PAYWALL_REPLACE", "0");
        std::env::set_var("NEWS_PAYWALL_SUMMARY", "0");
        std::env::set_var("NEWS_CROSS_DEDUP", "0");
        std::env::remove_var("NEWS_AI_THEME");
        std::env::remove_var("NULLCLAW_JOB_ID");
        Env { home, _guard: guard }
    }

    fn reply(&self, stdout: &str) -> &Env {
        std::fs::write(self.home.join("stub/resp.default"), stdout).unwrap();
        self
    }

    fn traces(&self, event: &str) -> Vec<serde_json::Value> {
        std::fs::read_to_string(self.home.join(".nullclaw/skill-traces.jsonl"))
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .filter(|v| v["event"] == event)
            .collect()
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

fn items() -> Vec<Item> {
    [
        ("台積電法說會釋出樂觀展望 - 經濟日報", G1),
        ("輝達財報優於市場預期 - 中央社", G2),
        ("播客節目談人工智慧發展 - 某電台", G3),
        ("華爾街分析聯準會利率路徑 - 華爾街日報", G4),
    ]
    .into_iter()
    .map(|(t, l)| Item {
        title: t.to_string(),
        source: "test-source".to_string(),
        link: l.to_string(),
        ..Default::default()
    })
    .collect()
}

fn verdict(action: Action, reason: Option<&'static str>, decoded: Option<&str>) -> Verdict {
    Verdict {
        action,
        reason,
        decoded_url: decoded.map(str::to_string),
        body_excerpt: None,
    }
}

/// #1 decoded and kept, #2 undecoded, #3 decoded but dropped as a podcast,
/// #4 decoded and paywalled.
fn seeded_cache() -> SharedCache {
    let cache = new_cache();
    {
        let mut c = cache.lock().unwrap();
        c.insert(G1.into(), verdict(Action::Keep, None, Some(P1)));
        c.insert(G2.into(), verdict(Action::Keep, Some("unresolved"), None));
        c.insert(G3.into(), verdict(Action::Drop, Some("media"), Some(P3)));
        c.insert(G4.into(), verdict(Action::TitleOnly, Some("paywalled"), Some(P4)));
    }
    cache
}

fn expected_decoded() -> Vec<String> {
    vec![
        format!("- 台積電法說會釋出樂觀展望 [🔗]({P1})"),
        format!("- 輝達財報優於市場預期 [🔗]({G2})"),
        format!("- 華爾街分析聯準會利率路徑 [🔗]({P4})  {PAYWALL_NOTE}"),
    ]
}

// ── the three section paths ──────────────────────────────────────────────────

#[test]
fn the_ai_section_renders_decoded_publisher_urls() {
    let _env = Env::new(true);
    let lines = run_ai_substage(&items(), 0, 4, DATE, &seeded_cache()).expect("ai substage");
    assert_eq!(lines, expected_decoded());
}

#[test]
fn a_default_section_renders_decoded_publisher_urls() {
    let _env = Env::new(true);
    let (lines, used_fallback) =
        summarize_default_section("general", &items(), DATE, &LinkMap::default(), &seeded_cache());
    assert!(!used_fallback);
    assert_eq!(lines, expected_decoded());
}

#[test]
fn a_custom_topic_renders_decoded_publisher_urls() {
    let _env = Env::new(true);
    let lines = run_custom_topic("半導體", &items(), DATE, &seeded_cache()).expect("custom topic");
    assert_eq!(lines, expected_decoded());
}

#[test]
fn with_the_precheck_off_every_pick_keeps_its_google_link() {
    // Nothing is decoded when Tier 2 does not run, so nothing is swapped —
    // even though the cache could have answered.
    let _env = Env::new(false);
    let lines = run_ai_substage(&items(), 0, 4, DATE, &seeded_cache()).expect("ai substage");
    assert_eq!(
        lines,
        vec![
            format!("- 台積電法說會釋出樂觀展望 [🔗]({G1})"),
            format!("- 輝達財報優於市場預期 [🔗]({G2})"),
            format!("- 播客節目談人工智慧發展 [🔗]({G3})"),
            format!("- 華爾街分析聯準會利率路徑 [🔗]({G4})"),
        ]
    );
}

#[test]
fn a_decoded_url_that_would_break_the_markdown_link_keeps_the_google_link() {
    let _env = Env::new(true);
    let cache = seeded_cache();
    cache.lock().unwrap().insert(
        G1.into(),
        verdict(Action::Keep, None, Some("https://en.example.org/wiki/Foo_(bar)")),
    );
    let lines = run_ai_substage(&items(), 0, 4, DATE, &cache).expect("ai substage");
    assert_eq!(lines[0], format!("- 台積電法說會釋出樂觀展望 [🔗]({G1})"));
}

#[test]
fn coverage_is_traced_once_per_precheck_call() {
    let env = Env::new(true);
    run_ai_substage(&items(), 0, 4, DATE, &seeded_cache()).expect("ai substage");
    let events = env.traces("links_decoded");
    assert_eq!(events.len(), 1, "{events:?}");
    // Three picks survive (#3 is dropped); #1 and #4 resolved.
    assert_eq!(events[0]["section"], "ai");
    assert_eq!(events[0]["decoded"], 2);
    assert_eq!(events[0]["total"], 3);
}

// ── the protected-name check after a section cache hit ───────────────────────
//
// The check joins a digest line back to its headline by link. A section served
// from the day cache skips the precheck, and the precheck memo is per process,
// so on the second and third run of the day nothing in memory knows that the
// cached line's publisher URL belongs to the Google link of its source.

const G5: &str = "https://news.google.com/rss/articles/CBMi-five?oc=5";
const P5: &str = "https://www.example.com/anthropic-claude-5";

/// One headline naming a protected company, rendered without the name.
fn protected_items(section: &str) -> Vec<(String, Vec<Item>)> {
    let item = Item {
        title: "Anthropic ships a new Claude model for enterprises - Example".to_string(),
        source: "test-source".to_string(),
        link: G5.to_string(),
        ..Default::default()
    };
    let mut all = vec![(section.to_string(), vec![item])];
    for key in ["ai", "tech", "general"] {
        if key != section {
            all.push((key.to_string(), Vec::new()));
        }
    }
    all
}

fn protected_cache() -> SharedCache {
    let cache = new_cache();
    cache
        .lock()
        .unwrap()
        .insert(G5.into(), verdict(Action::Keep, None, Some(P5)));
    cache
}

fn ctx() -> AlertContext {
    AlertContext::new(None, String::new(), None)
}

fn assert_loss_reported(events: &[serde_json::Value]) {
    for e in events {
        assert_eq!(e["names"], serde_json::json!(["Anthropic"]), "{e}");
        assert_eq!(e["links"], serde_json::json!([P5]), "{e}");
    }
}

#[test]
fn a_cached_ai_section_still_reports_a_lost_protected_name() {
    let env = Env::new(true);
    env.reply("- #1 新模型發布引發業界高度關注");
    let all = protected_items("ai");

    let first = summarize_llm(&all, &ctx(), DATE, &protected_cache()).expect("first run");
    assert!(first.contains(P5), "{first}");
    assert_eq!(env.traces("protected_name_lost").len(), 1);

    // A new process: fresh memo, section served from the day cache.
    let second = summarize_llm(&all, &ctx(), DATE, &new_cache()).expect("second run");
    assert!(second.contains(P5), "{second}");
    assert_eq!(env.traces("news_cache_hit").len(), 1);
    let losses = env.traces("protected_name_lost");
    assert_eq!(losses.len(), 2, "{losses:?}");
    assert_loss_reported(&losses);
}

#[test]
fn a_cached_custom_topic_still_reports_a_lost_protected_name() {
    let env = Env::new(true);
    env.reply("- #1 新模型發布引發業界高度關注");
    let all = protected_items("半導體");
    let topics = vec!["半導體".to_string()];

    let first = summarize_llm_custom(&all, &topics, &ctx(), DATE, &protected_cache());
    assert!(first.contains(P5), "{first}");
    assert_eq!(env.traces("protected_name_lost").len(), 1);

    let second = summarize_llm_custom(&all, &topics, &ctx(), DATE, &new_cache());
    assert!(second.contains(P5), "{second}");
    assert_eq!(env.traces("news_cache_hit").len(), 1);
    let losses = env.traces("protected_name_lost");
    assert_eq!(losses.len(), 2, "{losses:?}");
    assert_loss_reported(&losses);
}

#[test]
fn a_cache_entry_written_before_the_sidecar_existed_still_replays() {
    // Entries from before this change have no link sidecar. They must still
    // be served, and their Google links still join on the Google link.
    let env = Env::new(true);
    env.reply("- #1 新模型發布引發業界高度關注");
    let all = protected_items("半導體");
    let topics = vec!["半導體".to_string()];
    summarize_llm_custom(&all, &topics, &ctx(), DATE, &protected_cache());

    let day = env.home.join(".nullclaw/.news-cache/2026-11-17");
    for entry in std::fs::read_dir(&day).unwrap().flatten() {
        let path = entry.path();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if name.ends_with(".links.json") {
            std::fs::remove_file(&path).unwrap();
        } else {
            let body = std::fs::read_to_string(&path).unwrap().replace(P5, G5);
            std::fs::write(&path, body).unwrap();
        }
    }

    let second = summarize_llm_custom(&all, &topics, &ctx(), DATE, &new_cache());
    assert!(second.contains(G5), "{second}");
    let losses = env.traces("protected_name_lost");
    assert_eq!(losses.len(), 2, "{losses:?}");
    assert_eq!(losses[1]["links"], serde_json::json!([G5]));
}

// ── an entry is never written without its link sidecar ───────────────────────

/// The day's cache files, split into (entries, sidecars).
fn day_files(env: &Env) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let day = env.home.join(".nullclaw/.news-cache/2026-11-17");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&day)
        .map(|d| d.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    files.sort();
    files
        .into_iter()
        .partition(|p| !p.to_string_lossy().ends_with(".links.json"))
}

/// Run `section` once to learn its file names, then clear them and plant a
/// directory where the sidecar goes, so writing the sidecar fails.
fn block_sidecar(env: &Env, section: impl Fn()) -> PathBuf {
    section();
    let (entries, sidecars) = day_files(env);
    assert_eq!((entries.len(), sidecars.len()), (1, 1), "{entries:?} {sidecars:?}");
    std::fs::remove_file(&entries[0]).unwrap();
    std::fs::remove_file(&sidecars[0]).unwrap();
    std::fs::create_dir(&sidecars[0]).unwrap();
    entries[0].clone()
}

#[test]
fn an_ai_entry_whose_sidecar_cannot_be_written_is_not_cached() {
    let env = Env::new(true);
    let entry = block_sidecar(&env, || {
        run_ai_substage(&items(), 0, 4, DATE, &seeded_cache()).expect("ai substage");
    });
    let lines = run_ai_substage(&items(), 0, 4, DATE, &seeded_cache()).expect("ai substage");
    // The section itself still renders; only the cache write is skipped.
    assert_eq!(lines, expected_decoded());
    assert!(!entry.exists(), "entry written without its sidecar");
    assert_eq!(env.traces("news_cache_write_skipped").len(), 1);
}

#[test]
fn a_custom_topic_entry_whose_sidecar_cannot_be_written_is_not_cached() {
    let env = Env::new(true);
    let entry = block_sidecar(&env, || {
        run_custom_topic("半導體", &items(), DATE, &seeded_cache()).expect("custom topic");
    });
    let lines = run_custom_topic("半導體", &items(), DATE, &seeded_cache()).expect("custom topic");
    assert_eq!(lines, expected_decoded());
    assert!(!entry.exists(), "entry written without its sidecar");
    assert_eq!(env.traces("news_cache_write_skipped").len(), 1);
}

// ── the record of what the model wrote ───────────────────────────────────────

#[test]
fn a_default_section_records_each_pick_beside_its_source_title() {
    // Recorded before the precheck drops anything, so #3 is still there: the
    // record is of what the model answered, not of what shipped.
    let env = Env::new(true);
    summarize_default_section("general", &items(), DATE, &LinkMap::default(), &seeded_cache());
    let events = env.traces("section_picks");
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0]["section"], "general");
    assert_eq!(
        events[0]["picks"],
        serde_json::json!([
            {"n": 1, "source": "台積電法說會釋出樂觀展望 - 經濟日報", "written": "台積電法說會釋出樂觀展望"},
            {"n": 2, "source": "輝達財報優於市場預期 - 中央社", "written": "輝達財報優於市場預期"},
            {"n": 3, "source": "播客節目談人工智慧發展 - 某電台", "written": "播客節目談人工智慧發展"},
            {"n": 4, "source": "華爾街分析聯準會利率路徑 - 華爾街日報", "written": "華爾街分析聯準會利率路徑"},
        ])
    );
}

// ── what counts as a renderable URL ──────────────────────────────────────────

#[test]
fn only_a_plain_http_url_is_renderable() {
    assert_eq!(publisher_link(Some(P1)), Some(P1));
    assert_eq!(publisher_link(Some("http://a.example/x")), Some("http://a.example/x"));
    assert_eq!(publisher_link(Some("HTTPS://A.example/x")), Some("HTTPS://A.example/x"));
    assert_eq!(publisher_link(Some("  https://a.example/x \n")), Some("https://a.example/x"));
    assert_eq!(publisher_link(None), None);
    assert_eq!(publisher_link(Some("")), None);
    assert_eq!(publisher_link(Some("https://")), None);
    assert_eq!(publisher_link(Some("ftp://a.example/x")), None);
    assert_eq!(publisher_link(Some("javascript:alert(1)")), None);
    assert_eq!(publisher_link(Some("/relative/path")), None);
}

#[test]
fn a_url_that_would_end_the_markdown_link_early_is_not_renderable() {
    // `[🔗](…)` ends at the first `)`; whitespace ends it for Telegram too.
    assert_eq!(publisher_link(Some("https://a.example/Foo_(bar)")), None);
    assert_eq!(publisher_link(Some("https://a.example/a(b")), None);
    assert_eq!(publisher_link(Some("https://a.example/a b")), None);
    assert_eq!(publisher_link(Some("https://a.example/a\tb")), None);
    // A trailing backslash escapes the closing `)` of `[🔗](…\)`.
    assert_eq!(publisher_link(Some("https://a.example/path\\")), None);
    assert_eq!(publisher_link(Some("https://a.example/a\\b")), None);
}
