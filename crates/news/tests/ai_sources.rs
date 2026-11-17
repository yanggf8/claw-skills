//! `NEWS_AI_SOURCES` — delivery wiring for the direct AI feeds (spec
//! `docs/specs/2026-11-17-news-ai-direct-feeds-design.md` §5–§7, phase B).
//!
//! Offline. The model is a stub agent resolved through `$HOME` (the seam
//! `tests/pipeline.rs` documents); Google feeds are injected through
//! `run::FeedSource`; direct feeds and article pages are served by a local
//! stub that records every request and answers 418 to anything unscripted.
//! Precheck verdicts for Google links are pre-seeded in the shared cache,
//! keyed exactly as the real precheck keys them, so no decode is attempted.

use news::precheck::{new_cache, SharedCache};
use news::quality::{Action, Verdict};
use news::text::Item;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard};

const DATE: &str = "2026/11/17 (Tue)";

// HOME and every NEWS_* knob are process-wide: every test takes this lock for
// its whole body, and gets its own HOME so no day cache leaks across tests.
static ENV_LOCK: Mutex<()> = Mutex::new(());
static SEQ: AtomicUsize = AtomicUsize::new(0);

struct Env {
    home: PathBuf,
    _guard: MutexGuard<'static, ()>,
}

impl Env {
    fn new() -> Env {
        let guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = std::env::temp_dir().join(format!(
            "news-ai-sources-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join(".nullclaw")).unwrap();
        std::fs::create_dir_all(home.join("stub")).unwrap();
        let bin = home.join("nullclaw/zig-out/bin");
        std::fs::create_dir_all(&bin).unwrap();
        // resp.<n> for the nth call, else resp.default; each prompt recorded.
        std::fs::write(
            bin.join("nullclaw"),
            r#"#!/bin/sh
d="$HOME/stub"
n=$(cat "$d/counter" 2>/dev/null || echo 0)
n=$((n + 1))
echo "$n" > "$d/counter"
printf '%s' "$4" > "$d/prompt.$n"
if [ -f "$d/resp.$n" ]; then cat "$d/resp.$n"
elif [ -f "$d/resp.default" ]; then cat "$d/resp.default"
fi
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
        std::env::remove_var("CLAW_CONFIG");
        std::env::remove_var("CLAW_ENV");
        std::env::remove_var("NEWS_AI_SOURCES");
        std::env::set_var("NEWS_PRECHECK", "1");
        std::env::set_var("NEWS_PRECHECK_DECODE_TIMEOUT", "0.5");
        std::env::set_var("NEWS_PRECHECK_FETCH_TIMEOUT", "2");
        std::env::set_var("NEWS_PAYWALL_REPLACE", "0");
        std::env::set_var("NEWS_PAYWALL_SUMMARY", "0");
        std::env::set_var("NEWS_AI_THEME", "off");
        std::env::set_var("NEWS_CROSS_DEDUP", "0");
        std::env::set_var("NEWS_LLM_DEDUP_HINTS", "1");
        std::env::set_var("NEWS_LLM_POST_DEDUP", "1");
        Env {
            home,
            _guard: guard,
        }
    }

    fn reply(&self, n: usize, stdout: &str) {
        std::fs::write(self.home.join(format!("stub/resp.{n}")), stdout).unwrap();
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

fn item(title: &str, link: &str) -> Item {
    Item {
        title: title.to_string(),
        source: "test-source".to_string(),
        link: link.to_string(),
        ..Default::default()
    }
}

fn verdict(action: Action, reason: Option<&'static str>, decoded: Option<&str>) -> Verdict {
    Verdict {
        action,
        reason,
        decoded_url: decoded.map(str::to_string),
        body_excerpt: None,
    }
}

// ── the google fixture ───────────────────────────────────────────────────────

fn g(n: &str) -> String {
    format!("https://news.google.com/rss/articles/CBMi-{n}?oc=5")
}

fn google_ai() -> Vec<Item> {
    vec![
        item("台積電法說會釋出樂觀展望 - 經濟日報", &g("a1")),
        item("輝達財報優於市場預期 - 中央社", &g("a2")),
        item("歐盟通過人工智慧法案修正 - 聯合報", &g("a3")),
        item("華爾街分析聯準會利率路徑 - 華爾街日報", &g("a4")),
        item("百度發布新一代文心大模型 - 鉅亨網", &g("a5")),
        item("日本政府擴大算力補助計畫 - 日經中文網", &g("a6")),
    ]
}

fn google_tech() -> Vec<Item> {
    vec![
        item("美光擴大記憶體產能投資 - 工商時報", &g("t1")),
        item("三星晶圓代工良率提升 - 數位時代", &g("t2")),
        item("鴻海電動車新車款亮相 - 科技新報", &g("t3")),
    ]
}

fn google_general() -> Vec<Item> {
    vec![
        item("颱風路徑北轉氣象署發布警報 - 自由時報", &g("n1")),
        item("立法院三讀通過預算案修正 - 公視新聞", &g("n2")),
        item("國際油價連三日下跌 - 經濟日報", &g("n3")),
    ]
}

/// Every Google pick resolves from the memo: #a1 decoded and kept, #a2
/// undecoded, #a4 paywalled, the rest decoded.
fn google_cache() -> SharedCache {
    let cache = new_cache();
    {
        let mut c = cache.lock().unwrap();
        c.insert(
            g("a1"),
            verdict(Action::Keep, None, Some("https://money.udn.com/a1")),
        );
        c.insert(g("a2"), verdict(Action::Keep, Some("unresolved"), None));
        c.insert(
            g("a3"),
            verdict(Action::Keep, None, Some("https://udn.com/a3")),
        );
        c.insert(
            g("a4"),
            verdict(
                Action::TitleOnly,
                Some("paywalled"),
                Some("https://www.wsj.com/a4"),
            ),
        );
        c.insert(
            g("a5"),
            verdict(Action::Keep, None, Some("https://news.cnyes.com/a5")),
        );
        c.insert(
            g("a6"),
            verdict(Action::Keep, None, Some("https://zh.cn.nikkei.com/a6")),
        );
        for t in ["t1", "t2", "t3", "n1", "n2", "n3"] {
            c.insert(
                g(t),
                verdict(
                    Action::Keep,
                    None,
                    Some(&format!("https://pub.example.tw/{t}")),
                ),
            );
        }
    }
    cache
}

/// Four calls in order: AI half [0,3), AI half [3,6), tech, general.
fn google_replies(env: &Env) {
    env.reply(
        1,
        "- #1 台積電法說會釋出樂觀展望\n- #3 歐盟通過人工智慧法案修正",
    );
    env.reply(
        2,
        "- #1 華爾街分析聯準會利率路徑\n- #2 百度發布新一代文心大模型",
    );
    env.reply(
        3,
        "- #1 美光擴大記憶體產能投資\n- #2 三星晶圓代工良率提升\n- #3 鴻海電動車新車款亮相",
    );
    env.reply(
        4,
        "- #1 颱風路徑北轉氣象署發布警報\n- #2 立法院三讀通過預算案修正",
    );
}

fn google_all_items() -> Vec<(String, Vec<Item>)> {
    vec![
        ("ai".to_string(), google_ai()),
        ("tech".to_string(), google_tech()),
        ("general".to_string(), google_general()),
    ]
}

/// The digest `summarize_llm` produced for this fixture at 9a5db7b, before
/// any phase-B code existed — captured from that build, not written by hand.
const GOOGLE_GOLDEN: &str = "📰 早安新聞摘要 — 2026/11/17 (Tue)\n\n**🤖 AI 人工智慧**\n- 台積電法說會釋出樂觀展望 [🔗](https://money.udn.com/a1)\n- 歐盟通過人工智慧法案修正 [🔗](https://udn.com/a3)\n- 華爾街分析聯準會利率路徑 [🔗](https://www.wsj.com/a4)  ⚠\u{fe0f} 付費牆（原文需訂閱）\n- 百度發布新一代文心大模型 [🔗](https://news.cnyes.com/a5)\n\n**💻 科技 & 半導體**\n- 美光擴大記憶體產能投資 [🔗](https://pub.example.tw/t1)\n- 三星晶圓代工良率提升 [🔗](https://pub.example.tw/t2)\n- 鴻海電動車新車款亮相 [🔗](https://pub.example.tw/t3)\n\n**🌏 重大新聞**\n- 颱風路徑北轉氣象署發布警報 [🔗](https://pub.example.tw/n1)\n- 立法院三讀通過預算案修正 [🔗](https://pub.example.tw/n2)\n\nℹ\u{fe0f} 本次含 1 則付費牆新聞（原文需訂閱）";

#[test]
fn google_mode_digest_is_byte_identical_to_the_pre_change_build() {
    let env = Env::new();
    google_replies(&env);
    let ctx = news::alert::AlertContext::new(None, "main".into(), None);
    let digest = news::digest::summarize_llm(&google_all_items(), &ctx, DATE, &google_cache())
        .expect("digest");
    assert_eq!(digest, GOOGLE_GOLDEN);
}

// ── shared harness for the run-level tests ───────────────────────────────────

use news::alert::AlertContext;
use news::cli::DeliverArgs;
use news::config::AI_SUBSTAGE_CACHE_VARIANT;
use news::direct::{ConfigError, DirectReport};
use news::run::{run_deliver, FeedSource, LiveFeeds};
use news::sources::{parse_ai_sources, AiSources, AiSourcesSetting};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::{mpsc, Arc};
use std::time::Duration;

const JOB: &str = "job-ai-sources:7";

const TECH_GENERAL: &str = "**💻 科技 & 半導體**\n\
- 美光擴大記憶體產能投資 [🔗](https://pub.example.tw/t1)\n\
- 三星晶圓代工良率提升 [🔗](https://pub.example.tw/t2)\n\
- 鴻海電動車新車款亮相 [🔗](https://pub.example.tw/t3)\n\
\n\
**🌏 重大新聞**\n\
- 颱風路徑北轉氣象署發布警報 [🔗](https://pub.example.tw/n1)\n\
- 立法院三讀通過預算案修正 [🔗](https://pub.example.tw/n2)\n";

const TECH_REPLY: &str =
    "- #1 美光擴大記憶體產能投資\n- #2 三星晶圓代工良率提升\n- #3 鴻海電動車新車款亮相";
const GENERAL_REPLY: &str = "- #1 颱風路徑北轉氣象署發布警報\n- #2 立法院三讀通過預算案修正";

/// Google feeds from memory, recording every name asked for; direct feeds
/// through the real `LiveFeeds` path, so the operator's config file under
/// `$HOME` and the real fetch are what run.
#[derive(Default)]
struct TestFeeds {
    asked: Mutex<Vec<String>>,
}

impl TestFeeds {
    fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }
}

impl FeedSource for TestFeeds {
    fn builtin(&self, name: &str) -> Vec<Item> {
        self.asked.lock().unwrap().push(name.to_string());
        match name {
            "ai_us" => google_ai(),
            "tech" => google_tech(),
            "general" => google_general(),
            _ => Vec::new(),
        }
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

fn default_args() -> DeliverArgs {
    DeliverArgs {
        lang: "zh".into(),
        account: "main".into(),
        ..Default::default()
    }
}

/// `(exit code, stdout)` of one scheduled delivery to stdout.
fn deliver_run(feeds: &TestFeeds) -> (i32, String) {
    std::env::set_var("NULLCLAW_JOB_ID", JOB);
    let mut out: Vec<u8> = Vec::new();
    let code = run_deliver(&default_args(), feeds, DATE, &google_cache(), &mut out);
    std::env::remove_var("NULLCLAW_JOB_ID");
    (code, String::from_utf8(out).unwrap())
}

fn stdout_for(digest: &str) -> String {
    format!("{digest}\n\n`{JOB}`\n[skill-status:ok]\n[trace:{JOB}]\n")
}

/// `reason` of every block in the on-disk failure log, in order.
fn alert_reasons(env: &Env) -> Vec<String> {
    std::fs::read_to_string(env.home.join(".nullclaw/news-failures.log"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.strip_prefix("reason: ").map(str::to_string))
        .collect()
}

fn alert_details(env: &Env) -> Vec<String> {
    std::fs::read_to_string(env.home.join(".nullclaw/news-failures.log"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.strip_prefix("detail: ").map(str::to_string))
        .collect()
}

// ── a local publisher: feeds and article pages ───────────────────────────────

#[derive(Clone)]
struct Route {
    status: u16,
    body: String,
}

struct Publisher {
    base: String,
    seen: mpsc::Receiver<String>,
}

impl Publisher {
    /// Fails closed: an unscripted path answers 418. `script` receives the
    /// base URL, because a feed has to name its own article URLs.
    fn start(script: impl FnOnce(&str) -> Vec<(&'static str, Route)>) -> Publisher {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let base = format!("http://127.0.0.1:{port}");
        let routes: Arc<HashMap<String, Route>> = Arc::new(
            script(&base)
                .into_iter()
                .map(|(p, r)| (p.to_string(), r))
                .collect(),
        );
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                let routes = Arc::clone(&routes);
                let tx = tx.clone();
                std::thread::spawn(move || serve(stream, &routes, &tx));
            }
        });
        Publisher { base, seen: rx }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    /// Every request target (path and query) served so far, sorted.
    fn seen(&self) -> Vec<String> {
        let mut out = Vec::new();
        while let Ok(r) = self.seen.recv_timeout(Duration::from_millis(200)) {
            out.push(r);
        }
        out.sort();
        out
    }
}

fn serve(stream: TcpStream, routes: &HashMap<String, Route>, tx: &mpsc::Sender<String>) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() {
        return;
    }
    let target = request_line
        .split_whitespace()
        .nth(1)
        .unwrap_or("")
        .to_string();
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) if line == "\r\n" => break,
            Ok(_) => {}
            Err(_) => break,
        }
    }
    let _ = tx.send(target.clone());
    let path = target.split('?').next().unwrap_or("");
    let route = routes.get(path).cloned().unwrap_or(Route {
        status: 418,
        body: "unscripted".to_string(),
    });
    let mut out = stream;
    let _ = out.write_all(
        format!(
            "HTTP/1.1 {} X\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            route.status,
            route.body.len()
        )
        .as_bytes(),
    );
    let _ = out.write_all(route.body.as_bytes());
    let _ = out.flush();
}

fn ok(body: &str) -> Route {
    Route {
        status: 200,
        body: body.to_string(),
    }
}

/// RSS dated relative to the real clock: `LiveFeeds` filters on `now`.
fn fresh_rss(items: &[(&str, &str, i64)]) -> String {
    let mut s =
        String::from("<?xml version=\"1.0\"?><rss version=\"2.0\"><channel><title>t</title>");
    for (title, link, minutes_ago) in items {
        let at = jiff::Timestamp::now()
            .checked_sub(jiff::SignedDuration::from_mins(*minutes_ago))
            .unwrap();
        let date = at.strftime("%a, %d %b %Y %H:%M:%S +0000");
        s.push_str(&format!(
            "<item><title>{title}</title><link>{link}</link><pubDate>{date}</pubDate></item>"
        ));
    }
    s.push_str("</channel></rss>");
    s
}

const ARTICLE: &str = "<html><body><p>短文</p></body></html>";
const D1: &str = "微軟公布新一代人工智慧晶片";
const D2: &str = "工研院發表語言模型評測報告";

/// One feed of two fresh items, each with a readable article page.
fn publisher() -> Publisher {
    Publisher::start(|base| {
        let feed = fresh_rss(&[
            (D1, &format!("{base}/art/1"), 10),
            (D2, &format!("{base}/art/2"), 20),
        ]);
        vec![
            ("/ai.xml", ok(&feed)),
            ("/art/1", ok(ARTICLE)),
            ("/art/2", ok(ARTICLE)),
        ]
    })
}

fn write_feed_config(env: &Env, feeds: &[(&str, String)]) {
    let list: Vec<serde_json::Value> = feeds
        .iter()
        .map(|(name, url)| serde_json::json!({"name": name, "url": url, "max_items": 10}))
        .collect();
    std::fs::write(
        env.home.join(".nullclaw/news-ai-feeds.json"),
        serde_json::json!({"feeds": list, "max_age_hours": 30, "feeds_deadline_secs": 10})
            .to_string(),
    )
    .unwrap();
}

fn direct_ai_section(p: &Publisher) -> String {
    format!(
        "**🤖 AI 人工智慧**\n- {D1} [🔗]({})\n- {D2} [🔗]({})\n",
        p.url("/art/1"),
        p.url("/art/2")
    )
}

fn header() -> String {
    format!("📰 早安新聞摘要 — {DATE}\n\n")
}

/// Calls in order for a two-item direct AI section: half [0,1), half [1,2),
/// tech, general.
fn direct_replies(env: &Env) {
    env.reply(1, &format!("- #1 {D1}"));
    env.reply(2, &format!("- #1 {D2}"));
    env.reply(3, TECH_REPLY);
    env.reply(4, GENERAL_REPLY);
}

fn agent_calls(env: &Env) -> usize {
    std::fs::read_to_string(env.home.join("stub/counter"))
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

// ── §6 NEWS_AI_SOURCES ───────────────────────────────────────────────────────

#[test]
fn only_unset_or_empty_means_google_and_only_the_two_names_are_valid() {
    assert_eq!(parse_ai_sources(None), AiSourcesSetting::Google);
    assert_eq!(parse_ai_sources(Some("")), AiSourcesSetting::Google);
    assert_eq!(parse_ai_sources(Some("google")), AiSourcesSetting::Google);
    assert_eq!(parse_ai_sources(Some("direct")), AiSourcesSetting::Direct);
    for bad in ["direc", "Direct", " direct", "direct ", "GOOGLE", " "] {
        assert_eq!(
            parse_ai_sources(Some(bad)),
            AiSourcesSetting::Invalid(bad.to_string()),
            "{bad:?}"
        );
    }
}

#[test]
fn the_two_modes_never_share_an_ai_substage_cache_variant() {
    // R1: Google keeps today's variant, so this release invalidates nothing.
    assert_eq!(AiSources::Google.cache_variant(), AI_SUBSTAGE_CACHE_VARIANT);
    assert_ne!(AiSources::Direct.cache_variant(), AI_SUBSTAGE_CACHE_VARIANT);
    // A prompt bump renames the Google variant; the direct one is derived
    // from it so the same bump cannot leave direct replaying stale answers.
    assert!(
        AiSources::Direct
            .cache_variant()
            .starts_with(AI_SUBSTAGE_CACHE_VARIANT),
        "{}",
        AiSources::Direct.cache_variant()
    );
    assert_eq!(AiSources::Google.as_str(), "google");
    assert_eq!(AiSources::Direct.as_str(), "direct");
}

#[test]
fn unset_delivers_the_google_digest_byte_identical_with_marker_ok_and_no_alert() {
    let env = Env::new();
    google_replies(&env);
    let feeds = TestFeeds::default();
    let (code, out) = deliver_run(&feeds);
    assert_eq!(code, 0);
    assert_eq!(out, stdout_for(GOOGLE_GOLDEN));
    assert_eq!(alert_reasons(&env), Vec::<String>::new());
    assert_eq!(
        feeds.asked(),
        vec!["ai_us", "ai_labs", "ai_cn", "ai_tw", "tech", "general"]
    );
}

#[test]
fn a_typo_alerts_once_builds_google_and_keeps_the_marker_ok() {
    let env = Env::new();
    google_replies(&env);
    std::env::set_var("NEWS_AI_SOURCES", "direc");
    let feeds = TestFeeds::default();
    let (code, out) = deliver_run(&feeds);
    std::env::remove_var("NEWS_AI_SOURCES");

    assert_eq!(code, 0);
    assert_eq!(out, stdout_for(GOOGLE_GOLDEN));
    assert_eq!(alert_reasons(&env), vec!["news_ai_sources_invalid"]);
    assert_eq!(
        alert_details(&env),
        vec!["NEWS_AI_SOURCES=\"direc\" is not google|direct; built the AI section from google"]
    );
    assert_eq!(
        feeds.asked(),
        vec!["ai_us", "ai_labs", "ai_cn", "ai_tw", "tech", "general"]
    );
}

#[test]
fn the_variable_is_read_at_call_time_not_once_per_process() {
    let env = Env::new();
    google_replies(&env);
    std::env::set_var("NEWS_AI_SOURCES", "direct");
    // No config: direct reads nothing, so the Google AI feeds stay unasked.
    let feeds = TestFeeds::default();
    let _ = deliver_run(&feeds);
    std::env::remove_var("NEWS_AI_SOURCES");
    assert_eq!(feeds.asked(), vec!["direct", "tech", "general"]);
    drop(env);

    let env = Env::new();
    google_replies(&env);
    let feeds = TestFeeds::default();
    let _ = deliver_run(&feeds);
    assert_eq!(
        feeds.asked(),
        vec!["ai_us", "ai_labs", "ai_cn", "ai_tw", "tech", "general"]
    );
}

// ── §6 direct delivery ───────────────────────────────────────────────────────

#[test]
fn direct_builds_the_ai_section_from_the_operator_feeds_only() {
    let env = Env::new();
    direct_replies(&env);
    let p = publisher();
    write_feed_config(&env, &[("iThome", p.url("/ai.xml"))]);
    std::env::set_var("NEWS_AI_SOURCES", "direct");
    let feeds = TestFeeds::default();
    let (code, out) = deliver_run(&feeds);
    std::env::remove_var("NEWS_AI_SOURCES");

    assert_eq!(code, 0);
    assert_eq!(
        out,
        stdout_for(&format!(
            "{}{}\n{TECH_GENERAL}",
            header(),
            direct_ai_section(&p)
        ))
    );
    assert_eq!(feeds.asked(), vec!["direct", "tech", "general"]);
    assert_eq!(alert_reasons(&env), Vec::<String>::new());
    assert_eq!(env.traces("ai_direct_feeds").len(), 1);
}

#[test]
fn direct_with_no_config_alerts_and_ships_tech_and_general_with_marker_ok() {
    let env = Env::new();
    env.reply(1, TECH_REPLY);
    env.reply(2, GENERAL_REPLY);
    std::env::set_var("NEWS_AI_SOURCES", "direct");
    let feeds = TestFeeds::default();
    let (code, out) = deliver_run(&feeds);
    std::env::remove_var("NEWS_AI_SOURCES");

    assert_eq!(code, 0);
    assert_eq!(
        out,
        stdout_for(&format!(
            "{}**🤖 AI 人工智慧**\n- 今日無相關新聞\n\n{TECH_GENERAL}",
            header()
        ))
    );
    assert_eq!(alert_reasons(&env), vec!["ai_direct_feeds_empty"]);
    let path = env.home.join(".nullclaw/news-ai-feeds.json");
    assert_eq!(
        alert_details(&env),
        vec![format!(
            "direct AI feed config missing ({}: missing); AI section empty, google not used",
            path.display()
        )]
    );
    // Never a silent fallback to Google.
    assert_eq!(feeds.asked(), vec!["direct", "tech", "general"]);
    assert_eq!(agent_calls(&env), 2);
}

#[test]
fn direct_with_every_feed_failing_alerts_with_each_feed_outcome() {
    let env = Env::new();
    env.reply(1, TECH_REPLY);
    env.reply(2, GENERAL_REPLY);
    let p = Publisher::start(|base| {
        let stale = fresh_rss(&[("三年前的舊聞", &format!("{base}/art/9"), 60 * 24 * 365 * 3)]);
        vec![("/stale.xml", ok(&stale))]
    });
    write_feed_config(
        &env,
        &[("Dead", p.url("/gone.xml")), ("Old", p.url("/stale.xml"))],
    );
    std::env::set_var("NEWS_AI_SOURCES", "direct");
    let feeds = TestFeeds::default();
    let (code, out) = deliver_run(&feeds);
    std::env::remove_var("NEWS_AI_SOURCES");

    assert_eq!(code, 0);
    assert_eq!(
        out,
        stdout_for(&format!(
            "{}**🤖 AI 人工智慧**\n- 今日無相關新聞\n\n{TECH_GENERAL}",
            header()
        ))
    );
    assert_eq!(alert_reasons(&env), vec!["ai_direct_feeds_empty"]);
    assert_eq!(
        alert_details(&env),
        vec![
            "direct AI feeds kept 0 items; AI section empty, google not used. \
Dead=http_error(HTTP 418) fetched=0 kept=0; Old=ok fetched=1 stale=1 kept=0"
                .to_string()
        ]
    );
    assert_eq!(feeds.asked(), vec!["direct", "tech", "general"]);
}

// ── §5 precheck makes no Google request for a direct item ───────────────────

fn direct_items(p: &Publisher) -> Vec<Item> {
    vec![
        item(&format!("{D1} - iThome"), &p.url("/art/1")),
        item(&format!("{D2} - iThome"), &p.url("/art/2")),
    ]
}

#[test]
fn a_direct_pick_is_prechecked_on_its_own_url_with_no_decode() {
    let env = Env::new();
    env.reply(1, &format!("- #1 {D1}\n- #2 {D2}"));
    let p = publisher();
    let lines = news::summarize::run_ai_substage_for(
        AiSources::Direct,
        &direct_items(&p),
        0,
        2,
        DATE,
        &new_cache(),
    )
    .expect("substage");
    assert_eq!(
        lines,
        vec![
            format!("- {D1} [🔗]({})", p.url("/art/1")),
            format!("- {D2} [🔗]({})", p.url("/art/2")),
        ]
    );
    // A decode would first request the link with Google's edition params
    // appended (`google_news_article_url`), then POST to batchexecute; the
    // article fetch is the only request a direct pick may cost.
    assert_eq!(p.seen(), vec!["/art/1", "/art/2"]);
    let t2 = env.traces("quality_tier2");
    assert_eq!(t2.len(), 1);
    assert_eq!(t2[0]["checked"], 2);
}

#[test]
fn a_direct_pick_already_checked_this_run_is_not_fetched_again() {
    // Level 3 re-covers what Level 2 saw; the shared memo must serve it.
    let _env = Env::new();
    let p = publisher();
    let items = direct_items(&p);
    let numbered: news::select::NumberedMap = items
        .iter()
        .enumerate()
        .map(|(i, it)| (i as u32 + 1, news::render::Numbered::from(it)))
        .collect();
    let cache = new_cache();
    let summary = format!("- #1 {D1}\n- #2 {D2}");
    news::precheck::precheck_apply_for(&summary, &numbered, "ai", &cache, true);
    assert_eq!(p.seen(), vec!["/art/1", "/art/2"]);
    news::precheck::precheck_apply_for(&summary, &numbered, "ai", &cache, true);
    assert_eq!(p.seen(), Vec::<String>::new());
}

// ── R1 cache isolation ───────────────────────────────────────────────────────

fn day_dir(env: &Env) -> PathBuf {
    env.home.join(".nullclaw/.news-cache/2026-11-17")
}

const GOOGLE_CACHED: &str =
    "- 快取裡的谷歌條目 [🔗](https://news.google.com/rss/articles/CBMi-cached?oc=5)";

fn seed_google_entry(env: &Env, start: usize, end: usize) -> PathBuf {
    std::fs::create_dir_all(day_dir(env)).unwrap();
    let path = day_dir(env).join(format!(
        "{AI_SUBSTAGE_CACHE_VARIANT}-{start:03}-{end:03}.txt"
    ));
    std::fs::write(&path, GOOGLE_CACHED).unwrap();
    path
}

#[test]
fn a_direct_run_ignores_a_google_cache_entry_for_the_same_day_and_range() {
    let env = Env::new();
    direct_replies(&env);
    let a = seed_google_entry(&env, 0, 1);
    let b = seed_google_entry(&env, 1, 2);
    let p = publisher();
    write_feed_config(&env, &[("iThome", p.url("/ai.xml"))]);
    std::env::set_var("NEWS_AI_SOURCES", "direct");
    let (_, out) = deliver_run(&TestFeeds::default());
    std::env::remove_var("NEWS_AI_SOURCES");

    assert_eq!(
        out,
        stdout_for(&format!(
            "{}{}\n{TECH_GENERAL}",
            header(),
            direct_ai_section(&p)
        ))
    );
    assert_eq!(agent_calls(&env), 4);
    // Read under its own variant, written under its own variant: the Google
    // entries are untouched and the direct ones sit beside them.
    assert_eq!(std::fs::read_to_string(a).unwrap(), GOOGLE_CACHED);
    assert_eq!(std::fs::read_to_string(b).unwrap(), GOOGLE_CACHED);
    let direct = AiSources::Direct.cache_variant();
    assert_eq!(
        std::fs::read_to_string(day_dir(&env).join(format!("{direct}-000-001.txt"))).unwrap(),
        format!("- {D1} [🔗]({})", p.url("/art/1"))
    );
    assert_eq!(
        std::fs::read_to_string(day_dir(&env).join(format!("{direct}-001-002.txt"))).unwrap(),
        format!("- {D2} [🔗]({})", p.url("/art/2"))
    );
}

#[test]
fn the_translation_fallback_writes_under_the_direct_variant() {
    let env = Env::new();
    let a = seed_google_entry(&env, 0, 1);
    env.reply(1, "- #1 Microsoft unveils a new AI chip");
    env.reply(2, &format!("- #1 {D1}"));
    let p = publisher();
    let lines = news::summarize::run_ai_substage_for(
        AiSources::Direct,
        &direct_items(&p)[..1],
        0,
        1,
        DATE,
        &new_cache(),
    )
    .expect("substage");
    assert_eq!(lines, vec![format!("- {D1} [🔗]({})", p.url("/art/1"))]);
    assert_eq!(agent_calls(&env), 2);
    assert_eq!(std::fs::read_to_string(a).unwrap(), GOOGLE_CACHED);
    let direct = AiSources::Direct.cache_variant();
    assert_eq!(
        std::fs::read_to_string(day_dir(&env).join(format!("{direct}-000-001.txt"))).unwrap(),
        format!("- {D1} [🔗]({})", p.url("/art/1"))
    );
}

#[test]
fn the_level3_diagnostic_looks_up_the_direct_variant() {
    let env = Env::new();
    // Every call is rejected, so half [0,2) fails and so does its first
    // quarter; the alert then lists which quarters are cached. Google
    // entries exist for both quarters and must not be reported.
    std::fs::write(env.home.join("stub/resp.default"), "沒有編號的回覆").unwrap();
    seed_google_entry(&env, 0, 1);
    seed_google_entry(&env, 1, 2);
    let items = vec![
        item(
            "微軟公布新一代人工智慧晶片 - iThome",
            "https://www.ithome.com.tw/news/1",
        ),
        item(
            "工研院發表語言模型評測報告 - iThome",
            "https://www.ithome.com.tw/news/2",
        ),
        item(
            "台大團隊開源醫療影像模型 - iThome",
            "https://www.ithome.com.tw/news/3",
        ),
        item(
            "金管會發布金融業生成式指引 - iThome",
            "https://www.ithome.com.tw/news/4",
        ),
    ];
    let ctx = AlertContext::new(None, "main".into(), None);
    let r = news::summarize::summarize_default_ai_substaged_for(
        AiSources::Direct,
        &items,
        DATE,
        &ctx,
        &new_cache(),
    );
    assert!(r.is_err());
    assert_eq!(alert_reasons(&env), vec!["ai_substage_level3_failed"]);
    let detail = &alert_details(&env)[0];
    assert!(detail.ends_with("quarters cached so far: []"), "{detail}");
}

// ── §7 ai_section_rendered ───────────────────────────────────────────────────

use news::render::{ai_section_link_counts, count_ai_stories};

#[test]
fn stories_count_one_per_bullet_in_a_plain_section() {
    assert_eq!(
        count_ai_stories("- 甲 [🔗](https://a.tw/1)\n- 乙 [🔗](https://a.tw/2)\n- 丙"),
        3
    );
    // `render.rs::linked`'s bodyless, linkless bullet is a story too.
    assert_eq!(count_ai_stories("-\n- 甲"), 2);
}

#[test]
fn theme_headings_are_not_stories() {
    let themed = "▸ 產品發布\n- 甲 [🔗](https://a.tw/1)\n- 乙 [🔗](https://a.tw/2)\n▸ 政策監管\n- 丙 [🔗](https://a.tw/3)";
    assert_eq!(count_ai_stories(themed), 3);
}

#[test]
fn a_paywall_replacement_pair_is_one_story() {
    let pair = "- 免費替代報導 [🔗](https://free.tw/1)\n　↳ 原文：付費原文 [🔗](https://www.wsj.com/1)  ⚠️ 付費牆（原文需訂閱）\n- 乙 [🔗](https://a.tw/2)";
    assert_eq!(count_ai_stories(pair), 2);
}

#[test]
fn a_paywall_summary_with_three_continuations_is_one_story() {
    let summary = "- 付費報導 [🔗](https://www.ft.com/1)  ⚠️ 付費牆（原文需訂閱）\n　↳ 第一點\n　↳ 第二點\n　↳ 第三點";
    assert_eq!(count_ai_stories(summary), 1);
}

#[test]
fn the_empty_sentinel_is_zero_stories() {
    assert_eq!(count_ai_stories("- 今日無相關新聞"), 0);
}

#[test]
fn links_are_counted_with_the_google_hosted_ones_apart() {
    let section = "- 甲 [🔗](https://news.google.com/rss/articles/CBMi-x?oc=5)\n\
\u{3000}↳ 原文：乙 [🔗](https://www.wsj.com/1)  ⚠️ 付費牆（原文需訂閱）\n\
- 丙 [🔗](https://NEWS.GOOGLE.COM/rss/articles/y)\n\
- 丁 [🔗](https://news.google.com.example.org/z)\n\
- 戊";
    assert_eq!(ai_section_link_counts(section), (4, 2));
    // Each host rule on its own, so two wrong rules cannot cancel out.
    assert_eq!(
        ai_section_link_counts("- 丙 [🔗](https://NEWS.GOOGLE.COM/rss/articles/y)"),
        (1, 1)
    );
    assert_eq!(
        ai_section_link_counts("- 丁 [🔗](https://news.google.com.example.org/z)"),
        (1, 0)
    );
    assert_eq!(ai_section_link_counts("- 今日無相關新聞"), (0, 0));
}

#[test]
fn every_google_run_traces_one_ai_section_rendered_with_its_source() {
    let env = Env::new();
    google_replies(&env);
    let _ = deliver_run(&TestFeeds::default());
    let events = env.traces("ai_section_rendered");
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0]["stories"], 4);
    assert_eq!(events[0]["links_total"], 4);
    assert_eq!(events[0]["links_google"], 0);
    assert_eq!(events[0]["ai_sources"], "google");
    // The field belongs to this event only.
    for other in ["news_run_timing", "section_timing", "ai_substage_complete"] {
        for e in env.traces(other) {
            assert!(e.get("ai_sources").is_none(), "{other}: {e}");
        }
    }
}

#[test]
fn a_google_section_with_an_undecoded_pick_counts_its_google_link() {
    let env = Env::new();
    // #2 of the first half is a2, which the memo leaves undecoded.
    env.reply(
        1,
        "- #1 台積電法說會釋出樂觀展望\n- #2 輝達財報優於市場預期",
    );
    env.reply(2, "- #2 百度發布新一代文心大模型");
    env.reply(3, TECH_REPLY);
    env.reply(4, GENERAL_REPLY);
    let _ = deliver_run(&TestFeeds::default());
    let events = env.traces("ai_section_rendered");
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0]["stories"], 3);
    assert_eq!(events[0]["links_total"], 3);
    assert_eq!(events[0]["links_google"], 1);
}

#[test]
fn a_direct_run_traces_its_section_as_direct() {
    let env = Env::new();
    direct_replies(&env);
    let p = publisher();
    write_feed_config(&env, &[("iThome", p.url("/ai.xml"))]);
    std::env::set_var("NEWS_AI_SOURCES", "direct");
    let _ = deliver_run(&TestFeeds::default());
    std::env::remove_var("NEWS_AI_SOURCES");
    let events = env.traces("ai_section_rendered");
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0]["stories"], 2);
    assert_eq!(events[0]["links_total"], 2);
    assert_eq!(events[0]["links_google"], 0);
    assert_eq!(events[0]["ai_sources"], "direct");
}

#[test]
fn a_direct_outage_traces_an_empty_section() {
    let env = Env::new();
    env.reply(1, TECH_REPLY);
    env.reply(2, GENERAL_REPLY);
    std::env::set_var("NEWS_AI_SOURCES", "direct");
    let _ = deliver_run(&TestFeeds::default());
    std::env::remove_var("NEWS_AI_SOURCES");
    let events = env.traces("ai_section_rendered");
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0]["stories"], 0);
    assert_eq!(events[0]["links_total"], 0);
    assert_eq!(events[0]["links_google"], 0);
    assert_eq!(events[0]["ai_sources"], "direct");
}

// ── §7 ai_direct_feeds when the config itself fails ─────────────────────────

/// The event with the four envelope fields every trace line carries removed.
fn direct_feeds_payload(env: &Env) -> Vec<serde_json::Value> {
    env.traces("ai_direct_feeds")
        .into_iter()
        .map(|mut v| {
            let o = v.as_object_mut().unwrap();
            for k in ["ts", "job_id", "skill", "event"] {
                assert!(o.remove(k).is_some(), "{k} missing");
            }
            v
        })
        .collect()
}

#[test]
fn a_missing_config_still_traces_ai_direct_feeds() {
    let env = Env::new();
    env.reply(1, TECH_REPLY);
    env.reply(2, GENERAL_REPLY);
    std::env::set_var("NEWS_AI_SOURCES", "direct");
    let _ = deliver_run(&TestFeeds::default());
    std::env::remove_var("NEWS_AI_SOURCES");
    assert_eq!(
        direct_feeds_payload(&env),
        vec![serde_json::json!({"config_error": "missing", "feeds": [], "total_kept": 0})]
    );
}

#[test]
fn an_invalid_config_still_traces_ai_direct_feeds() {
    let env = Env::new();
    env.reply(1, TECH_REPLY);
    env.reply(2, GENERAL_REPLY);
    std::fs::write(env.home.join(".nullclaw/news-ai-feeds.json"), "{not json").unwrap();
    std::env::set_var("NEWS_AI_SOURCES", "direct");
    let _ = deliver_run(&TestFeeds::default());
    std::env::remove_var("NEWS_AI_SOURCES");
    assert_eq!(
        direct_feeds_payload(&env),
        vec![serde_json::json!({"config_error": "invalid", "feeds": [], "total_kept": 0})]
    );
    assert_eq!(alert_reasons(&env), vec!["ai_direct_feeds_empty"]);
}

#[test]
fn level3_recovering_direct_picks_reuses_their_precheck_within_the_run() {
    // Half [0,2) passes the precheck, then fails its language gate and the
    // translation retry; Level 3 re-covers the same two items. Their
    // articles were already checked this run, so nothing is fetched again.
    let env = Env::new();
    const D3: &str = "台大團隊開源醫療影像模型";
    const D4: &str = "金管會發布金融業生成式指引";
    env.reply(1, "- #1 Microsoft unveils a new AI chip\n- #2 ITRI publishes a model benchmark");
    env.reply(2, "沒有編號的回覆");
    env.reply(3, &format!("- #1 {D3}\n- #2 {D4}"));
    env.reply(4, &format!("- #1 {D1}"));
    env.reply(5, &format!("- #1 {D2}"));
    let p = Publisher::start(|_| {
        vec![
            ("/art/1", ok(ARTICLE)),
            ("/art/2", ok(ARTICLE)),
            ("/art/3", ok(ARTICLE)),
            ("/art/4", ok(ARTICLE)),
        ]
    });
    let items = vec![
        item(&format!("{D1} - iThome"), &p.url("/art/1")),
        item(&format!("{D2} - iThome"), &p.url("/art/2")),
        item(&format!("{D3} - iThome"), &p.url("/art/3")),
        item(&format!("{D4} - iThome"), &p.url("/art/4")),
    ];
    let ctx = AlertContext::new(None, "main".into(), None);
    let lines = news::summarize::summarize_default_ai_substaged_for(
        AiSources::Direct,
        &items,
        DATE,
        &ctx,
        &new_cache(),
    )
    .expect("section");

    assert_eq!(
        lines,
        vec![
            format!("- {D1} [🔗]({})", p.url("/art/1")),
            format!("- {D2} [🔗]({})", p.url("/art/2")),
            format!("- {D3} [🔗]({})", p.url("/art/3")),
            format!("- {D4} [🔗]({})", p.url("/art/4")),
        ]
    );
    assert_eq!(agent_calls(&env), 5);
    assert_eq!(env.traces("ai_substage_level3_start").len(), 1);
    assert_eq!(p.seen(), vec!["/art/1", "/art/2", "/art/3", "/art/4"]);
}
