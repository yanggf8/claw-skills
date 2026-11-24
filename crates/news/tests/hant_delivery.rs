//! Simplified → Traditional at the two points where text reaches a reader:
//! the delivered digest (default and custom topics) and the shadow file.
//!
//! Offline, through the stub agent of `tests/support`. Google picks resolve
//! from a pre-seeded precheck cache, so nothing is decoded or fetched; the
//! seeded publisher URLs carry raw Simplified paths, which must survive.

mod support;

use news::cli::DeliverArgs;
use news::direct::{ConfigError, DirectReport};
use news::precheck::{new_cache, SharedCache};
use news::quality::{Action, Verdict};
use news::run::{run_deliver, FeedSource, LiveFeeds};
use news::shadow::run_shadow_ai;
use news::text::Item;
use serde_json::json;
use std::path::Path;
use support::*;

struct Feeds;

impl FeedSource for Feeds {
    fn builtin(&self, name: &str) -> Vec<Item> {
        match name {
            "ai_us" => items("a", &[D1, D2]),
            "tech" => items("t", &["美光擴大記憶體產能投資", "三星晶圓代工良率提升"]),
            "general" => items(
                "n",
                &["光電交鋒 蔣萬安：卓揆心中只有烏賊", "國際油價連三日下跌"],
            ),
            _ => Vec::new(),
        }
    }
    fn topic(&self, _: &str) -> Vec<Item> {
        items(
            "c",
            &["沈伯洋夜訪凱道 蔣萬安陣營卡位", "台積電法說會釋出樂觀展望"],
        )
    }
    fn direct_ai(&self) -> Result<(Vec<Item>, DirectReport), ConfigError> {
        LiveFeeds.direct_ai()
    }
}

fn google(prefix: &str, n: usize) -> String {
    format!("https://news.google.com/rss/articles/CBMi-{prefix}{n}?oc=5")
}

fn publisher_url(prefix: &str, n: usize) -> String {
    format!("https://pub.example.tw/乌贼/{prefix}{n}")
}

fn items(prefix: &str, titles: &[&str]) -> Vec<Item> {
    titles
        .iter()
        .enumerate()
        .map(|(i, t)| Item {
            title: t.to_string(),
            source: "test-source".to_string(),
            link: google(prefix, i + 1),
            ..Default::default()
        })
        .collect()
}

fn cache() -> SharedCache {
    let cache = new_cache();
    for prefix in ["a", "t", "n", "c"] {
        for n in 1..=2 {
            cache.lock().unwrap().insert(
                google(prefix, n),
                Verdict {
                    action: Action::Keep,
                    reason: None,
                    decoded_url: Some(publisher_url(prefix, n)),
                    body_excerpt: None,
                },
            );
        }
    }
    cache
}

/// Calls in order: AI half [0,1), AI half [1,2), tech, general.
fn replies(env: &Env, ai_1: &str, general_1: &str) {
    env.reply(1, &format!("- #1 {ai_1}"));
    env.reply(2, &format!("- #1 {D2}"));
    env.reply(3, "- #1 美光擴大記憶體產能投資\n- #2 三星晶圓代工良率提升");
    env.reply(4, &format!("- #1 {general_1}\n- #2 國際油價連三日下跌"));
}

fn deliver(topics: Option<&str>) -> (i32, String) {
    let args = DeliverArgs {
        lang: "zh".into(),
        account: "main".into(),
        topics: topics.map(str::to_string),
        ..Default::default()
    };
    std::env::set_var("NULLCLAW_JOB_ID", JOB);
    let mut out: Vec<u8> = Vec::new();
    let code = run_deliver(&args, &Feeds, DATE, &cache(), &mut out);
    std::env::remove_var("NULLCLAW_JOB_ID");
    (code, String::from_utf8(out).unwrap())
}

fn digest(ai_1: &str, general_1: &str) -> String {
    let line = |t: &str, p: &str, n| format!("- {t} [🔗]({})", publisher_url(p, n));
    [
        format!("📰 早安新聞摘要 — {DATE}\n"),
        "**🤖 AI 人工智慧**".into(),
        line(ai_1, "a", 1),
        line(D2, "a", 2),
        "\n**💻 科技 & 半導體**".into(),
        line("美光擴大記憶體產能投資", "t", 1),
        line("三星晶圓代工良率提升", "t", 2),
        "\n**🌏 重大新聞**".into(),
        line(general_1, "n", 1),
        line("國際油價連三日下跌", "n", 2),
        format!("\n\n`{JOB}`\n{}", markers("ok")),
    ]
    .join("\n")
}

/// Every event's payload, without the fields a run stamps on each line.
fn conversions(env: &Env) -> Vec<serde_json::Value> {
    env.traces("simplified_converted")
        .into_iter()
        .map(|mut v| {
            for k in ["ts", "job_id", "skill", "event"] {
                v.as_object_mut().unwrap().remove(k);
            }
            v
        })
        .collect()
}

/// The text of every file under the day cache, in one string.
fn cached_text(dir: &Path) -> String {
    let mut all = String::new();
    for e in std::fs::read_dir(dir).unwrap().filter_map(Result::ok) {
        if e.path().is_dir() {
            all.push_str(&cached_text(&e.path()));
        } else {
            all.push_str(&std::fs::read_to_string(e.path()).unwrap());
        }
    }
    all
}

const AI_WRITTEN: &str = "微軟公布新一代人工智慧晶片即将出现";
const AI_READ: &str = "微軟公布新一代人工智慧晶片即將出現";
const GENERAL_WRITTEN: &str = "光電布局争议延烧，蔣万安质疑行政院长卓荣泰「心中只有乌贼」";
const GENERAL_READ: &str = "光電布局爭議延燒，蔣萬安質疑行政院長卓榮泰「心中只有烏賊」";

#[test]
fn a_default_digest_is_delivered_in_traditional() {
    let env = Env::new();
    replies(&env, AI_WRITTEN, GENERAL_WRITTEN);
    assert_eq!(deliver(None), (0, digest(AI_READ, GENERAL_READ)));
    assert_eq!(
        conversions(&env),
        vec![json!({
            "lines": 2,
            "chars": 11,
            "samples": [
                {"before": format!("- {AI_WRITTEN}"), "after": format!("- {AI_READ}")},
                {"before": format!("- {GENERAL_WRITTEN}"), "after": format!("- {GENERAL_READ}")},
            ],
        })]
    );
    let rendered = &env.traces("ai_section_rendered")[0];
    assert_eq!(rendered["stories"], 2);
    assert_eq!(rendered["links_total"], 2);
    assert_eq!(rendered["links_google"], 0);
}

#[test]
fn the_cache_keeps_what_the_model_wrote() {
    let env = Env::new();
    replies(&env, AI_WRITTEN, GENERAL_WRITTEN);
    deliver(None);
    let cached = cached_text(&env.home.join(".nullclaw/.news-cache"));
    assert!(cached.contains(AI_WRITTEN), "{cached}");
    assert!(!cached.contains(AI_READ), "{cached}");
}

#[test]
fn a_clean_digest_is_delivered_unchanged_and_traces_nothing() {
    let env = Env::new();
    replies(&env, D1, "蔣萬安：公布布局，里程碑獲准");
    assert_eq!(
        deliver(None),
        (0, digest(D1, "蔣萬安：公布布局，里程碑獲准"))
    );
    assert_eq!(conversions(&env), Vec::<serde_json::Value>::new());
    assert_eq!(env.traces("simplified_convert_unavailable").len(), 0);
}

#[test]
fn a_custom_topic_digest_is_delivered_in_traditional() {
    let env = Env::new();
    env.reply_default(
        "- #1 沈伯洋选前夜现身北市府前，蔣万安阵营抢先卡位凯道\n- #2 台積電法說會釋出樂觀展望",
    );
    let (code, out) = deliver(Some("台北市长"));
    assert_eq!(code, 0);
    assert_eq!(
        out,
        format!(
            "📰 每日新聞摘要 — {DATE}\n\n**台北市長**\n\
- 沈伯洋選前夜現身北市府前，蔣萬安陣營搶先卡位凱道 [🔗]({})\n\
- 台積電法說會釋出樂觀展望 [🔗]({})\n\n\n`{JOB}`\n{}",
            publisher_url("c", 1),
            publisher_url("c", 2),
            markers("ok")
        )
    );
    let traced = conversions(&env);
    assert_eq!(traced.len(), 1);
    assert_eq!(traced[0]["lines"], 2);
    assert_eq!(traced[0]["chars"], 8);
}

#[test]
fn the_shadow_file_is_written_in_traditional() {
    let env = Env::new();
    let p = publisher();
    env.write_feed_config(&[("iThome", p.url("/ai.xml"))]);
    env.reply(1, &format!("- #1 {AI_WRITTEN}"));
    env.reply(2, &format!("- #1 {D2}"));
    std::env::set_var("NULLCLAW_JOB_ID", JOB);
    let mut out: Vec<u8> = Vec::new();
    let code = run_shadow_ai(&LiveFeeds, DATE, &new_cache(), &mut out);
    std::env::remove_var("NULLCLAW_JOB_ID");
    assert_eq!((code, String::from_utf8(out).unwrap()), (0, markers("ok")));
    assert_eq!(
        std::fs::read_to_string(env.shadow_file()).unwrap(),
        two_story_section(&p).replace(D1, AI_READ)
    );
    let traced = conversions(&env);
    assert_eq!(traced.len(), 1);
    assert_eq!(traced[0]["lines"], 1);
    assert_eq!(traced[0]["chars"], 2);
    assert_eq!(env.traces("ai_sources_shadow")[0]["stories"], 2);
}

#[test]
fn a_clean_shadow_file_traces_nothing() {
    let env = Env::new();
    let p = publisher();
    env.write_feed_config(&[("iThome", p.url("/ai.xml"))]);
    two_story_replies(&env);
    let mut out: Vec<u8> = Vec::new();
    run_shadow_ai(&LiveFeeds, DATE, &new_cache(), &mut out);
    assert_eq!(
        std::fs::read_to_string(env.shadow_file()).unwrap(),
        two_story_section(&p)
    );
    assert_eq!(conversions(&env), Vec::<serde_json::Value>::new());
}
