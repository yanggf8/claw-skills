//! Direct AI feeds, offline half: config, parsing, Techmeme unwrap, the title
//! invariant, filter-then-cap and link validation (spec
//! `docs/specs/2026-11-17-news-ai-direct-feeds-design.md` §1–§3).
//!
//! Nothing here touches the network. Every test still runs under a scratch
//! `HOME`, so a stray trace write can never reach the operator's real
//! `~/.nullclaw/skill-traces.jsonl`.

use jiff::Timestamp;
use news::direct::{
    load_direct_config, parse_naive_offset, process_body, select_entries, ConfigError,
    DirectConfig, FeedCounts, FeedKind, FeedOutcome, FeedReport, FeedSpec,
};
use news::feed::{parse_feed_all, parse_feed_all_at, parse_feed_date_at, parse_rss, FeedEntry};
use news::text::{extract_source_name, title_without_source, Item};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;

fn scratch_home() -> &'static PathBuf {
    static HOME: OnceLock<PathBuf> = OnceLock::new();
    HOME.get_or_init(|| {
        let home = std::env::temp_dir().join(format!("news-direct-parse-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join(".nullclaw")).unwrap();
        std::env::set_var("HOME", &home);
        home
    })
}

/// One file per call — tests in this binary run concurrently.
fn write_config(body: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = scratch_home().join("configs");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("feeds-{}.json", N.fetch_add(1, Ordering::SeqCst)));
    std::fs::write(&path, body).unwrap();
    path
}

fn now() -> Timestamp {
    "2026-11-17T08:50:00Z".parse().unwrap()
}

fn plain(name: &str, max_items: usize) -> FeedSpec {
    FeedSpec {
        name: name.into(),
        url: "https://example.com/feed".into(),
        kind: FeedKind::Plain,
        max_items,
        naive_date_offset: None,
    }
}

fn entry(title: &str, link: &str, date: &str) -> FeedEntry {
    FeedEntry {
        title: title.into(),
        link: link.into(),
        date: date.into(),
        description: String::new(),
    }
}

fn item(title: &str, link: &str, date: &str, source: &str) -> Item {
    Item {
        title: title.into(),
        link: link.into(),
        pub_date: date.into(),
        decoded_url: None,
        source: source.into(),
    }
}

// ── §1 config ────────────────────────────────────────────────────────────────

#[test]
fn config_path_is_the_operators_file_under_home() {
    let home = scratch_home();
    assert_eq!(
        news::config::ai_feeds_file(),
        home.join(".nullclaw/news-ai-feeds.json")
    );
}

#[test]
fn config_loads_the_spec_example_exactly() {
    let path = write_config(
        r#"{
          "feeds": [
            {"name": "Techmeme", "url": "https://www.techmeme.com/feed.xml", "kind": "techmeme", "max_items": 15},
            {"name": "OpenAI", "url": "https://openai.com/news/rss.xml", "max_items": 5},
            {"name": "iThome", "url": "https://www.ithome.com.tw/rss/ai", "kind": "plain", "max_items": 10, "naive_date_offset": "+08:00"}
          ],
          "max_age_hours": 30,
          "feeds_deadline_secs": 40
        }"#,
    );
    let cfg = load_direct_config(Some(&path)).unwrap();
    assert_eq!(
        cfg,
        DirectConfig {
            feeds: vec![
                FeedSpec {
                    name: "Techmeme".into(),
                    url: "https://www.techmeme.com/feed.xml".into(),
                    kind: FeedKind::Techmeme,
                    max_items: 15,
                    naive_date_offset: None,
                },
                FeedSpec {
                    name: "OpenAI".into(),
                    url: "https://openai.com/news/rss.xml".into(),
                    kind: FeedKind::Plain,
                    max_items: 5,
                    naive_date_offset: None,
                },
                FeedSpec {
                    name: "iThome".into(),
                    url: "https://www.ithome.com.tw/rss/ai".into(),
                    kind: FeedKind::Plain,
                    max_items: 10,
                    naive_date_offset: Some("+08:00".into()),
                },
            ],
            max_age_hours: 30.0,
            feeds_deadline_secs: 40.0,
        }
    );
}

#[test]
fn config_default_path_reads_home() {
    let home = scratch_home();
    std::fs::write(
        home.join(".nullclaw/news-ai-feeds.json"),
        r#"{"feeds":[{"name":"A","url":"https://a.example/rss","max_items":1}],
            "max_age_hours":2,"feeds_deadline_secs":3}"#,
    )
    .unwrap();
    let cfg = load_direct_config(None).unwrap();
    assert_eq!(cfg.feeds, vec![FeedSpec {
        name: "A".into(),
        url: "https://a.example/rss".into(),
        kind: FeedKind::Plain,
        max_items: 1,
        naive_date_offset: None,
    }]);
}

#[test]
fn config_missing_file_is_missing() {
    let path = scratch_home().join("configs/does-not-exist.json");
    let err = load_direct_config(Some(&path)).unwrap_err();
    assert_eq!(err, ConfigError::Missing { path: path.clone() });
    assert_eq!(err.code(), "missing");
}

#[test]
fn config_empty_feed_list_is_empty() {
    for body in [
        r#"{"feeds": [], "max_age_hours": 30, "feeds_deadline_secs": 40}"#,
        "",
        "  \n",
    ] {
        let path = write_config(body);
        let err = load_direct_config(Some(&path)).unwrap_err();
        assert_eq!(err, ConfigError::Empty { path: path.clone() }, "body: {body:?}");
        assert_eq!(err.code(), "empty");
    }
}

fn invalid_detail(body: &str) -> String {
    let path = write_config(body);
    match load_direct_config(Some(&path)).unwrap_err() {
        ConfigError::Invalid { path: p, detail } => {
            assert_eq!(p, path);
            detail
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
}

#[test]
fn config_unparseable_shapes_are_invalid() {
    // Serde writes these details; only the variant is pinned.
    for body in [
        "{",
        "[]",
        r#"{"feeds": [{"name":"A","url":"https://a.example/rss","max_items":1}], "feeds_deadline_secs": 40}"#,
        r#"{"feeds": [{"name":"A","url":"https://a.example/rss"}], "max_age_hours": 30, "feeds_deadline_secs": 40}"#,
        r#"{"feeds": [{"name":"A","url":"https://a.example/rss","max_items":1,"kind":"atom"}], "max_age_hours": 30, "feeds_deadline_secs": 40}"#,
        r#"{"feeds": [{"name":"A","url":"https://a.example/rss","max_item":1}], "max_age_hours": 30, "feeds_deadline_secs": 40}"#,
        // Complete, plus a field the loader does not know: a misplaced
        // per-feed override must not be silently ignored.
        r#"{"feeds": [{"name":"A","url":"https://a.example/rss","max_items":1,"max_age_hours":2}], "max_age_hours": 30, "feeds_deadline_secs": 40}"#,
        r#"{"feeds": [{"name":"A","url":"https://a.example/rss","max_items":1}], "max_age_hours": 30, "feeds_deadline_secs": 40, "feed_timeout_secs": 5}"#,
    ] {
        invalid_detail(body);
    }
}

#[test]
fn config_semantic_violations_name_the_field() {
    let cases = [
        (
            r#"{"feeds":[{"name":"  ","url":"https://a.example/rss","max_items":1}],"max_age_hours":30,"feeds_deadline_secs":40}"#,
            "feeds[0].name is empty",
        ),
        (
            r#"{"feeds":[{"name":" A","url":"https://a.example/rss","max_items":1}],"max_age_hours":30,"feeds_deadline_secs":40}"#,
            "feeds[0].name \" A\" has surrounding whitespace",
        ),
        (
            r#"{"feeds":[{"name":"Ars - Tech","url":"https://a.example/rss","max_items":1}],"max_age_hours":30,"feeds_deadline_secs":40}"#,
            "feeds[0].name \"Ars - Tech\" contains \" - \", which would break the \"<headline> - <Source>\" title invariant",
        ),
        (
            r#"{"feeds":[{"name":"A","url":"https://a.example/1","max_items":1},{"name":"A","url":"https://a.example/2","max_items":1}],"max_age_hours":30,"feeds_deadline_secs":40}"#,
            "feeds[1].name \"A\" duplicates feeds[0]",
        ),
        (
            r#"{"feeds":[{"name":"A","url":"ftp://a.example/rss","max_items":1}],"max_age_hours":30,"feeds_deadline_secs":40}"#,
            "feeds[0].url \"ftp://a.example/rss\" is not an absolute http(s) URL",
        ),
        (
            r#"{"feeds":[{"name":"A","url":"https://a.example/rss","max_items":0}],"max_age_hours":30,"feeds_deadline_secs":40}"#,
            "feeds[0].max_items must be at least 1",
        ),
        (
            r#"{"feeds":[{"name":"A","url":"https://a.example/rss","max_items":1}],"max_age_hours":0,"feeds_deadline_secs":40}"#,
            "max_age_hours must be a positive number, got 0.0",
        ),
        (
            r#"{"feeds":[{"name":"A","url":"https://a.example/rss","max_items":1}],"max_age_hours":30,"feeds_deadline_secs":-1}"#,
            "feeds_deadline_secs must be a positive number of seconds, got -1.0",
        ),
        (
            r#"{"feeds":[{"name":"A","url":"https://a.example/rss","max_items":1}],"max_age_hours":30,"feeds_deadline_secs":1e300}"#,
            "feeds_deadline_secs must be a positive number of seconds, got 1e300",
        ),
    ];
    for (body, want) in cases {
        assert_eq!(invalid_detail(body), want, "body: {body}");
    }
}

// ── §4 parsing ───────────────────────────────────────────────────────────────

const ATOM: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title>The Verge - AI</title>
  <link rel="self" href="https://www.theverge.com/rss/ai/index.xml"/>
  <updated>2026-11-17T08:00:00-05:00</updated>
  <entry>
    <source><title>Nested title must not win</title></source>
    <title type="html">OpenAI ships a thing &amp; more</title>
    <link rel="replies" href="https://www.theverge.com/c/1"/>
    <link rel="alternate" type="text/html" href="https://www.theverge.com/ai/1/openai-ships"/>
    <updated>2026-11-17T07:30:00-05:00</updated>
    <published>2026-11-17T07:00:00-05:00</published>
    <author><name>Jane</name></author>
  </entry>
  <entry>
    <link href="https://www.theverge.com/ai/2/no-rel"/>
    <link rel="alternate" href="https://www.theverge.com/ai/2/second"/>
    <title><![CDATA[Second <entry> & CDATA]]></title>
    <updated>2026-11-17T06:00:00Z</updated>
  </entry>
</feed>"#;

#[test]
fn atom_entries_parse_with_href_links_and_published_first() {
    scratch_home();
    assert_eq!(
        parse_feed_all(ATOM).unwrap(),
        vec![
            entry(
                "OpenAI ships a thing & more",
                "https://www.theverge.com/ai/1/openai-ships",
                "2026-11-17T07:00:00-05:00",
            ),
            entry(
                "Second <entry> & CDATA",
                "https://www.theverge.com/ai/2/no-rel",
                "2026-11-17T06:00:00Z",
            ),
        ]
    );
}

const GOOGLE_SHAPED_RSS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:media="http://search.yahoo.com/mrss/"><channel><title>Google News</title>
<item><title>Headline A &amp; B - Reuters</title><link>https://news.google.com/rss/articles/CBMi1</link><guid isPermaLink="false">g1</guid><pubDate>Mon, 16 Nov 2026 22:00:00 GMT</pubDate><description>&lt;a href="https://x"&gt;y&lt;/a&gt;</description><source url="https://www.reuters.com">Reuters</source></item>
<item><title><![CDATA[CDATA &amp; literal - 經濟日報]]></title><link>https://news.google.com/rss/articles/CBMi2</link><pubDate>Mon, 16 Nov 2026 21:00:00 GMT</pubDate></item>
<item><title>Third - Bloomberg</title><link>https://news.google.com/rss/articles/CBMi3</link><pubDate>Mon, 16 Nov 2026 20:00:00 GMT</pubDate></item>
</channel></rss>"#;

#[test]
fn rss_parity_with_the_google_parser() {
    scratch_home();
    let google: Vec<(String, String, String)> = parse_rss(GOOGLE_SHAPED_RSS, 100)
        .into_iter()
        .map(|i| (i.title, i.link, i.pub_date))
        .collect();
    let direct: Vec<(String, String, String)> = parse_feed_all(GOOGLE_SHAPED_RSS)
        .unwrap()
        .into_iter()
        .map(|e| (e.title, e.link, e.date))
        .collect();
    assert_eq!(direct, google);
    assert_eq!(
        direct,
        vec![
            (
                "Headline A & B - Reuters".to_string(),
                "https://news.google.com/rss/articles/CBMi1".to_string(),
                "Mon, 16 Nov 2026 22:00:00 GMT".to_string(),
            ),
            (
                "CDATA &amp; literal - 經濟日報".to_string(),
                "https://news.google.com/rss/articles/CBMi2".to_string(),
                "Mon, 16 Nov 2026 21:00:00 GMT".to_string(),
            ),
            (
                "Third - Bloomberg".to_string(),
                "https://news.google.com/rss/articles/CBMi3".to_string(),
                "Mon, 16 Nov 2026 20:00:00 GMT".to_string(),
            ),
        ]
    );
}

#[test]
fn rss_description_is_captured() {
    scratch_home();
    let entries = parse_feed_all(GOOGLE_SHAPED_RSS).unwrap();
    assert_eq!(entries[0].description, r#"<a href="https://x">y</a>"#);
    assert_eq!(entries[1].description, "");
}

#[test]
fn parse_all_has_no_early_stop() {
    scratch_home();
    let mut xml = String::from("<rss><channel>");
    for i in 0..40 {
        xml.push_str(&format!(
            "<item><title>T{i}</title><link>https://e.example/{i}</link></item>"
        ));
    }
    xml.push_str("</channel></rss>");
    let entries = parse_feed_all(&xml).unwrap();
    assert_eq!(entries.len(), 40);
    assert_eq!(entries[39].link, "https://e.example/39");
}

#[test]
fn not_a_feed_is_a_parse_error() {
    scratch_home();
    assert_eq!(
        parse_feed_all("<html><body><p>Automated queries</p></body></html>").unwrap_err(),
        "root element <html> is not rss, feed or RDF"
    );
    assert!(parse_feed_all("<rss><channel><item><title>x</item></channel></rss>").is_err());
    assert_eq!(parse_feed_all("").unwrap_err(), "no root element");
}

#[test]
fn valid_feed_with_no_items_is_ok_and_empty() {
    scratch_home();
    assert_eq!(
        parse_feed_all("<rss><channel><title>quiet</title></channel></rss>").unwrap(),
        Vec::<FeedEntry>::new()
    );
}

// ── §2 the title invariant ───────────────────────────────────────────────────

#[test]
fn inner_dash_survives_title_without_source() {
    scratch_home();
    let (items, counts) = select_entries(
        &plain("The Verge", 5),
        vec![entry(
            "  OpenAI - Microsoft\n   deal reshaped  ",
            "https://www.theverge.com/ai/1",
            "Tue, 17 Nov 2026 08:00:00 GMT",
        )],
        30.0,
        now(),
    );
    assert_eq!(
        items,
        vec![item(
            "OpenAI - Microsoft deal reshaped - The Verge",
            "https://www.theverge.com/ai/1",
            "Tue, 17 Nov 2026 08:00:00 GMT",
            "The Verge",
        )]
    );
    assert_eq!(title_without_source(&items[0].title), "OpenAI - Microsoft deal reshaped");
    assert_eq!(extract_source_name(&items[0].title), "The Verge");
    assert_eq!(counts, FeedCounts { fetched: 1, kept: 1, ..Default::default() });
}

#[test]
fn empty_headline_is_malformed() {
    scratch_home();
    let (items, counts) = select_entries(
        &plain("X", 5),
        vec![
            entry("   ", "https://e.example/1", "Tue, 17 Nov 2026 08:00:00 GMT"),
            entry("Real", "https://e.example/2", "Tue, 17 Nov 2026 08:00:00 GMT"),
        ],
        30.0,
        now(),
    );
    assert_eq!(items.len(), 1);
    assert_eq!(counts, FeedCounts { fetched: 2, malformed: 1, kept: 1, ..Default::default() });
}

// ── §2 Techmeme ──────────────────────────────────────────────────────────────

fn techmeme_spec() -> FeedSpec {
    FeedSpec {
        name: "Techmeme".into(),
        url: "https://www.techmeme.com/feed.xml".into(),
        kind: FeedKind::Techmeme,
        max_items: 15,
        naive_date_offset: None,
    }
}

#[test]
fn techmeme_fixture_unwraps_to_publisher_items() {
    scratch_home();
    let xml = include_str!("fixtures/techmeme-2026-11-17.xml");
    let entries = parse_feed_all(xml).unwrap();
    let (items, counts) = select_entries(&techmeme_spec(), entries, 30.0, now());
    assert_eq!(
        items,
        vec![
            item(
                "A review of 300+ Chinese documents shows AI distillation has moved from academic experiments to widespread industrial use, with Beijing encouraging the practice - Reuters",
                "https://www.reuters.com/world/china/china-taps-us-ai-models-train-homegrown-rivals-chip-curbs-bite-2026-11-17/",
                "Tue, 17 Nov 2026 02:30:01 -0500",
                "Reuters",
            ),
            item(
                "Nvidia unveils its Rubin HPC GPU with \"more than 50,000\" CUDA cores, up from 28,672 in the current Rubin GPU, for agentic workflows and scientific simulations - Tom's Hardware",
                "https://www.tomshardware.com/pc-components/gpus/nvidia-announces-rubin-hpc-gpu-with-record-number-of-cuda-cores-at-supercomputing-2026-new-chip-accelerates-agentic-hpc-workloads-arrives-in-2028",
                "Tue, 17 Nov 2026 01:20:00 -0500",
                "Tom's Hardware",
            ),
            item(
                "Analysis: Amazon's $17B+ planned investment in Ohio data centers is at risk after midterm voters passed restriction measures affecting three proposed projects - Financial Times",
                "https://www.ft.com/content/e95adad4-b428-477d-9716-92172bbd199f",
                "Tue, 17 Nov 2026 00:40:01 -0500",
                "Financial Times",
            ),
            item(
                "Sources: Jensen Huang's daughter remains in a senior Nvidia role; before Jensen Huang resigned, she was his de facto chief of staff with considerable authority - New York Times",
                "https://www.nytimes.com/2026/11/16/technology/nvidia-jensen-madison-huang.html?unlocked_article_code=1.RFE.d0Ui.v_uJvV3zdxPR&smid=nytcore-ios-share",
                "Mon, 16 Nov 2026 22:20:00 -0500",
                "New York Times",
            ),
        ]
    );
    assert_eq!(counts, FeedCounts { fetched: 4, kept: 4, ..Default::default() });
}

fn tm_entry(title: &str, description: &str) -> FeedEntry {
    FeedEntry {
        title: title.into(),
        link: "https://www.techmeme.com/261117/p9#a261117p9".into(),
        date: "Tue, 17 Nov 2026 03:00:00 -0500".into(),
        description: description.into(),
    }
}

#[test]
fn techmeme_malformed_items_are_dropped_and_counted() {
    scratch_home();
    let article = r#"<A HREF="https://www.techmeme.com/x"><IMG></A><A HREF="https://www.wsj.com/tech/a-1?x=1&amp;y=2">h</A>"#;
    let (items, counts) = select_entries(
        &techmeme_spec(),
        vec![
            // no attribution at all
            tm_entry("A headline with no attribution", article),
            // attribution with an empty publication
            tm_entry("Headline (Jane Doe/)", article),
            // nothing but the attribution
            tm_entry("(Jane Doe/WSJ)", article),
            // only Techmeme links: no article to point at
            tm_entry(
                "Only permalinks (Jane Doe/WSJ)",
                r#"<A HREF="https://www.techmeme.com/261117/p9">p</A><A HREF="https://techmeme.com/">t</A>"#,
            ),
            // a well-formed one, with a parenthesis inside the headline
            tm_entry("Apple (AAPL) beats estimates (Jane Doe/Wall Street Journal)", article),
            // a parenthesis inside the attribution itself
            tm_entry("Sony raises outlook (Taro Yamada/Nikkei (Japan))", article),
        ],
        30.0,
        now(),
    );
    assert_eq!(
        items,
        vec![
        item(
            "Apple (AAPL) beats estimates - Wall Street Journal",
            "https://www.wsj.com/tech/a-1?x=1&y=2",
            "Tue, 17 Nov 2026 03:00:00 -0500",
            "Wall Street Journal",
        ),
        item(
            "Sony raises outlook - Nikkei (Japan)",
            "https://www.wsj.com/tech/a-1?x=1&y=2",
            "Tue, 17 Nov 2026 03:00:00 -0500",
            "Nikkei (Japan)",
        )]
    );
    assert_eq!(
        counts,
        FeedCounts { fetched: 6, malformed: 3, invalid_link: 1, kept: 2, ..Default::default() }
    );
}

#[test]
fn techmeme_skips_a_publication_homepage_link() {
    // An item with no lead image has the outlet's homepage as its first
    // non-Techmeme HREF; the article link is the bold headline after it.
    scratch_home();
    let desc = r#"<P><A HREF="https://www.techmeme.com/261117/p1#a261117p1" TITLE="Techmeme permalink"><IMG></A> Jane / <A HREF="https://www.ft.com/">Financial Times</A>:<BR><SPAN><B><A HREF="https://www.ft.com/content/abc">Head</A></B></SPAN></P>"#;
    let (items, _) = select_entries(
        &techmeme_spec(),
        vec![tm_entry("Head (Jane/Financial Times)", desc)],
        30.0,
        now(),
    );
    assert_eq!(items[0].link, "https://www.ft.com/content/abc");
}

#[test]
fn techmeme_attribution_without_author_is_the_publication() {
    scratch_home();
    let (items, _) = select_entries(
        &techmeme_spec(),
        vec![tm_entry(
            "Something happened (Reuters)",
            r#"<A HREF="https://www.reuters.com/a/b">x</A>"#,
        )],
        30.0,
        now(),
    );
    assert_eq!(items[0].title, "Something happened - Reuters");
    assert_eq!(items[0].source, "Reuters");
}

// ── §3 filter, then cap ──────────────────────────────────────────────────────

#[test]
fn stale_and_undated_ahead_of_a_fresh_item_do_not_crowd_it_out() {
    scratch_home();
    let (items, counts) = select_entries(
        &plain("OpenAI", 1),
        vec![
            entry("Old 1", "https://openai.com/1", "Mon, 02 Jan 2023 10:00:00 GMT"),
            entry("No date", "https://openai.com/2", ""),
            entry("Old 2", "https://openai.com/3", "2024-05-01T00:00:00Z"),
            entry("Garbled date", "https://openai.com/4", "yesterday-ish"),
            entry("Fresh", "https://openai.com/5", "Tue, 17 Nov 2026 07:00:00 GMT"),
        ],
        30.0,
        now(),
    );
    assert_eq!(
        items,
        vec![item(
            "Fresh - OpenAI",
            "https://openai.com/5",
            "Tue, 17 Nov 2026 07:00:00 GMT",
            "OpenAI",
        )]
    );
    assert_eq!(
        counts,
        FeedCounts { fetched: 5, stale: 2, undated: 2, kept: 1, ..Default::default() }
    );
}

#[test]
fn survivors_sort_newest_first_then_cap() {
    scratch_home();
    let (items, counts) = select_entries(
        &plain("Ars", 2),
        vec![
            entry("Older", "https://ars.example/1", "Tue, 17 Nov 2026 01:00:00 GMT"),
            entry("Newest", "https://ars.example/2", "2026-11-17T08:00:00Z"),
            entry("Middle", "https://ars.example/3", "Tue, 17 Nov 2026 04:00:00 +0000"),
        ],
        30.0,
        now(),
    );
    let titles: Vec<&str> = items.iter().map(|i| i.title.as_str()).collect();
    assert_eq!(titles, vec!["Newest - Ars", "Middle - Ars"]);
    assert_eq!(counts, FeedCounts { fetched: 3, capped: 1, kept: 2, ..Default::default() });
}

#[test]
fn age_boundary_is_inclusive() {
    scratch_home();
    // now = 08:50:00Z; 30 h earlier = 2026-11-16T02:50:00Z.
    let (items, counts) = select_entries(
        &plain("B", 5),
        vec![
            entry("At the limit", "https://b.example/1", "2026-11-16T02:50:00Z"),
            entry("One second over", "https://b.example/2", "2026-11-16T02:49:59Z"),
        ],
        30.0,
        now(),
    );
    let titles: Vec<&str> = items.iter().map(|i| i.title.as_str()).collect();
    assert_eq!(titles, vec!["At the limit - B"]);
    assert_eq!(counts, FeedCounts { fetched: 2, stale: 1, kept: 1, ..Default::default() });
}

// ── §2 [R6] link validation ──────────────────────────────────────────────────

#[test]
fn invalid_links_are_dropped_and_counted() {
    scratch_home();
    let d = "Tue, 17 Nov 2026 08:00:00 GMT";
    let (items, counts) = select_entries(
        &plain("V", 10),
        vec![
            entry("empty", "", d),
            entry("relative", "/2026/11/17/story", d),
            entry("ftp", "ftp://files.example/story", d),
            entry("javascript", "javascript:alert(1)", d),
            entry("parens", "https://v.example/a_(b)", d),
            entry("backslash", "https://v.example/a\\b", d),
            entry("no host", "https:///path-only", d),
            entry("ok", "https://v.example/real", d),
        ],
        30.0,
        now(),
    );
    assert_eq!(items, vec![item("ok - V", "https://v.example/real", d, "V")]);
    assert_eq!(counts, FeedCounts { fetched: 8, invalid_link: 7, kept: 1, ..Default::default() });
}

// ── review 2026-11-17: future-dated entries ─────────────────────────────────

#[test]
fn far_future_entries_ahead_of_a_fresh_one_do_not_crowd_it_out() {
    scratch_home();
    let (items, counts) = select_entries(
        &plain("Clock", 2),
        vec![
            entry("Year ahead", "https://c.example/1", "2027-11-17T08:00:00Z"),
            entry("Two hours ahead", "https://c.example/2", "Tue, 17 Nov 2026 10:50:00 GMT"),
            entry("Fresh", "https://c.example/3", "Tue, 17 Nov 2026 08:00:00 GMT"),
        ],
        30.0,
        now(),
    );
    assert_eq!(
        items,
        vec![item("Fresh - Clock", "https://c.example/3", "Tue, 17 Nov 2026 08:00:00 GMT", "Clock")]
    );
    assert_eq!(counts, FeedCounts { fetched: 3, future: 2, kept: 1, ..Default::default() });
}

#[test]
fn future_tolerance_boundary() {
    scratch_home();
    assert_eq!(news::direct::FUTURE_TOLERANCE, std::time::Duration::from_secs(3600));
    // now = 08:50:00Z; the tolerance ends at 09:50:00Z.
    let (items, counts) = select_entries(
        &plain("Edge", 5),
        vec![
            entry("At the tolerance", "https://e.example/1", "2026-11-17T09:50:00Z"),
            entry("One second past it", "https://e.example/2", "2026-11-17T09:50:01Z"),
        ],
        30.0,
        now(),
    );
    let titles: Vec<&str> = items.iter().map(|i| i.title.as_str()).collect();
    assert_eq!(titles, vec!["At the tolerance - Edge"]);
    assert_eq!(counts, FeedCounts { fetched: 2, future: 1, kept: 1, ..Default::default() });
}

#[test]
fn slightly_future_entry_does_not_outrank_a_current_one() {
    // Within tolerance the sort key is clamped to `now`, so a 30-minute-ahead
    // entry ties with one published exactly now and the feed's own order
    // decides; unclamped it would sort first and take the only slot.
    scratch_home();
    let (items, counts) = select_entries(
        &plain("Skew", 1),
        vec![
            entry("Current", "https://s.example/1", "2026-11-17T08:50:00Z"),
            entry("Thirty minutes ahead", "https://s.example/2", "2026-11-17T09:20:00Z"),
        ],
        30.0,
        now(),
    );
    let titles: Vec<&str> = items.iter().map(|i| i.title.as_str()).collect();
    assert_eq!(titles, vec!["Current - Skew"]);
    assert_eq!(counts, FeedCounts { fetched: 2, capped: 1, kept: 1, ..Default::default() });
}

// ── review 2026-11-17: truncated documents ──────────────────────────────────

#[test]
fn truncated_documents_are_parse_errors() {
    scratch_home();
    let cases = [
        (
            "<rss><channel><item><title>A</title><link>https://a.example/1</link></item>",
            "truncated: 2 element(s) still open at end of input",
        ),
        (
            "<rss><channel><item><title>A</title><link>https://a.example/1",
            "truncated: 4 element(s) still open at end of input",
        ),
        (
            "<feed xmlns=\"http://www.w3.org/2005/Atom\"><entry><title>A</title><link href=\"https://a.example/1\"/><updated>2026-11-17T08:00:00Z</updated></entry>",
            "truncated: 1 element(s) still open at end of input",
        ),
        (
            "<feed xmlns=\"http://www.w3.org/2005/Atom\"><entry><title>A</title><link href=\"https://a.example/1\"/>",
            "truncated: 2 element(s) still open at end of input",
        ),
    ];
    for (xml, want) in cases {
        assert_eq!(parse_feed_all(xml), Err(want.to_string()), "xml: {xml}");
    }
}

// ── review 2026-11-17: dates as real feeds write them ───────────────────────

#[test]
fn feed_dates_parse_to_the_instant() {
    scratch_home();
    let at = |s: &str| -> Timestamp { s.parse().unwrap() };
    let cases = [
        ("Tue, 17 Nov 2026 08:00:00 GMT", Some(at("2026-11-17T08:00:00Z"))),
        ("Tue, 17 Nov 2026 08:00:00 UT", Some(at("2026-11-17T08:00:00Z"))),
        ("Tue, 17 Nov 2026 03:00:00 EST", Some(at("2026-11-17T08:00:00Z"))),
        ("Tue, 17 Nov 2026 04:00:00 EDT", Some(at("2026-11-17T08:00:00Z"))),
        ("Tue, 17 Nov 2026 02:00:00 CST", Some(at("2026-11-17T08:00:00Z"))),
        ("Tue, 17 Nov 2026 00:00:00 PST", Some(at("2026-11-17T08:00:00Z"))),
        ("Tue, 17 Nov 2026 01:00:00 PDT", Some(at("2026-11-17T08:00:00Z"))),
        ("Tue, 17 Nov 2026 16:00:00 +0800", Some(at("2026-11-17T08:00:00Z"))),
        ("Tue, 17 Nov 2026 08:00 GMT", Some(at("2026-11-17T08:00:00Z"))),
        ("Tue, 3 Nov 2026 08:00:00 GMT", Some(at("2026-11-03T08:00:00Z"))),
        ("17 Nov 2026 08:00:00 +0000", Some(at("2026-11-17T08:00:00Z"))),
        ("  Tue, 17 Nov 2026 08:00:00 GMT\n", Some(at("2026-11-17T08:00:00Z"))),
        ("2026-11-17T16:00:00+08:00", Some(at("2026-11-17T08:00:00Z"))),
        ("2026-11-17T08:00:00.250Z", Some(at("2026-11-17T08:00:00.250Z"))),
        ("", None),
        ("yesterday", None),
        ("2026-11-17", None),
    ];
    for (raw, want) in cases {
        assert_eq!(news::direct::parse_feed_date(raw), want, "raw: {raw:?}");
    }
}

#[test]
fn ordering_is_by_instant_across_offsets() {
    scratch_home();
    let (items, _) = select_entries(
        &plain("Tz", 5),
        vec![
            entry("07:00Z", "https://t.example/1", "2026-11-17T15:00:00+08:00"),
            entry("08:00Z", "https://t.example/2", "2026-11-17T08:00:00Z"),
            entry("07:30Z", "https://t.example/3", "Tue, 17 Nov 2026 15:30:00 +0800"),
            entry("07:45Z", "https://t.example/4", "Tue, 17 Nov 2026 02:45:00 EST"),
        ],
        30.0,
        now(),
    );
    let titles: Vec<&str> = items.iter().map(|i| i.title.as_str()).collect();
    assert_eq!(titles, vec!["08:00Z - Tz", "07:45Z - Tz", "07:30Z - Tz", "07:00Z - Tz"]);
}

#[test]
fn atom_unreadable_published_falls_back_to_updated() {
    // The first date that parses wins, in the order pubDate, published,
    // updated. When none parses the first non-empty one is kept as written,
    // so the entry is still counted `undated` rather than silently dateless.
    scratch_home();
    let xml = r#"<feed xmlns="http://www.w3.org/2005/Atom">
      <entry><title>A</title><link href="https://a.example/1"/>
        <published>last Tuesday</published><updated>2026-11-17T07:30:00Z</updated></entry>
      <entry><title>B</title><link href="https://a.example/2"/>
        <published>2026-11-17T06:00:00Z</published><updated>2026-11-17T07:30:00Z</updated></entry>
      <entry><title>C</title><link href="https://a.example/3"/>
        <published>last Tuesday</published><updated>soon</updated></entry>
    </feed>"#;
    let dates: Vec<String> = parse_feed_all(xml).unwrap().into_iter().map(|e| e.date).collect();
    assert_eq!(dates, vec!["2026-11-17T07:30:00Z", "2026-11-17T06:00:00Z", "last Tuesday"]);
}

// ── naive_date_offset (spec §1, 2026-11-17) ─────────────────────────────────

#[test]
fn naive_offset_accepts_quarter_hours_within_fourteen() {
    scratch_home();
    for (raw, secs) in [
        ("+08:00", 8 * 3600),
        ("-09:30", -(9 * 3600 + 30 * 60)),
        ("+05:45", 5 * 3600 + 45 * 60),
        ("+14:00", 14 * 3600),
        ("-14:00", -14 * 3600),
        ("+00:00", 0),
        ("-00:00", 0),
    ] {
        assert_eq!(
            parse_naive_offset(raw).map(|o| o.seconds()),
            Some(secs),
            "raw: {raw:?}"
        );
    }
    for raw in [
        "", "Z", "08:00", "+8:00", "+0800", "+08", "+08:00:00", " +08:00", "+08:00 ", "+15:00",
        "+14:15", "-14:30", "+08:10", "+08:60", "UTC+8", "+０８:00", "+08000", "+08.00",
    ] {
        assert_eq!(parse_naive_offset(raw), None, "raw: {raw:?}");
    }
}

#[test]
fn config_rejects_an_invalid_naive_offset_naming_feed_and_field() {
    for (offset, idx_body) in [("+8:00", 1), ("+15:00", 1), ("+08:10", 1), ("08:00", 1)] {
        let body = format!(
            r#"{{"feeds":[{{"name":"A","url":"https://a.example/rss","max_items":1,"naive_date_offset":"+08:00"}},
                        {{"name":"iThome","url":"https://i.example/rss","max_items":1,"naive_date_offset":"{offset}"}}],
                "max_age_hours":30,"feeds_deadline_secs":40}}"#
        );
        assert_eq!(
            invalid_detail(&body),
            format!(
                "feeds[{idx_body}].naive_date_offset \"{offset}\" must be ±HH:MM, a whole quarter hour between -14:00 and +14:00"
            )
        );
    }
    // Not a string at all: serde's own message, so only the variant is pinned.
    invalid_detail(
        r#"{"feeds":[{"name":"A","url":"https://a.example/rss","max_items":1,"naive_date_offset":8}],"max_age_hours":30,"feeds_deadline_secs":40}"#,
    );
}

#[test]
fn zoneless_dates_use_the_declared_offset_and_only_then() {
    scratch_home();
    let at = |s: &str| -> Timestamp { s.parse().unwrap() };
    let plus8 = parse_naive_offset("+08:00");
    let cases = [
        ("2026-11-17 16:29", plus8, Some(at("2026-11-17T08:29:00Z"))),
        ("2026-11-17 16:29:30", plus8, Some(at("2026-11-17T08:29:30Z"))),
        ("2026-11-17T16:29", plus8, Some(at("2026-11-17T08:29:00Z"))),
        ("2026-11-17T16:29:30", plus8, Some(at("2026-11-17T08:29:30Z"))),
        (" 2026-11-17 16:29\n", plus8, Some(at("2026-11-17T08:29:00Z"))),
        ("2026-11-17 00:15", parse_naive_offset("-09:30"), Some(at("2026-11-17T09:45:00Z"))),
        // A date that carries its own zone keeps it; the offset is ignored.
        ("2026-11-17T16:29:00Z", plus8, Some(at("2026-11-17T16:29:00Z"))),
        ("2026-11-17T16:29:00+09:00", plus8, Some(at("2026-11-17T07:29:00Z"))),
        ("Tue, 17 Nov 2026 16:29:00 GMT", plus8, Some(at("2026-11-17T16:29:00Z"))),
        // Zoneless without a declared offset stays undated.
        ("2026-11-17 16:29", None, None),
        ("2026-11-17T16:29:30", None, None),
        // Not the zoneless shape, offset or not.
        ("2026-11-17", plus8, None),
        ("2026-11-17 16", plus8, None),
        ("2026-11-17 16:29:30.5", plus8, None),
        ("2026/11/17 16:29", plus8, None),
        ("2026-11-17 25:00", plus8, None),
        ("2026-02-30 10:00", plus8, None),
        ("2026-11-17  16:29", plus8, None),
    ];
    for (raw, offset, want) in cases {
        assert_eq!(parse_feed_date_at(raw, offset), want, "raw: {raw:?}, offset: {offset:?}");
    }
}

#[test]
fn first_parseable_date_considers_the_offset() {
    scratch_home();
    let xml = r#"<feed xmlns="http://www.w3.org/2005/Atom">
      <entry><title>A</title><link href="https://a.example/1"/>
        <published>2026-11-17 06:00</published><updated>2026-11-17T07:30:00Z</updated></entry>
    </feed>"#;
    let date = |offset| parse_feed_all_at(xml, offset).unwrap()[0].date.clone();
    assert_eq!(date(None), "2026-11-17T07:30:00Z");
    assert_eq!(date(parse_naive_offset("+08:00")), "2026-11-17 06:00");
    // The plain entry point is the no-offset one.
    assert_eq!(parse_feed_all(xml).unwrap()[0].date, "2026-11-17T07:30:00Z");
}

fn ithome(offset: Option<&str>) -> FeedSpec {
    FeedSpec {
        name: "iThome".into(),
        url: "https://www.ithome.com.tw/rss/ai".into(),
        kind: FeedKind::Plain,
        max_items: 10,
        naive_date_offset: offset.map(str::to_string),
    }
}

/// Recorded 2026-11-17T09:23:09Z (17:23 Taipei); the newest item read 16:29.
fn ithome_now() -> Timestamp {
    "2026-11-17T09:23:09Z".parse().unwrap()
}

#[test]
fn ithome_fixture_with_its_declared_offset_is_kept_at_the_right_instants() {
    scratch_home();
    let body = include_bytes!("fixtures/ithome-ai-2026-11-17.xml");
    let (report, items) = process_body(&ithome(Some("+08:00")), body, 30.0, ithome_now());
    assert_eq!(
        report,
        FeedReport {
            name: "iThome".into(),
            outcome: FeedOutcome::Ok,
            detail: None,
            counts: FeedCounts { fetched: 3, kept: 3, ..Default::default() },
        }
    );
    let got: Vec<(String, String, Option<Timestamp>)> = items
        .iter()
        .map(|i| (i.title.clone(), i.link.clone(), parse_feed_date_at(&i.pub_date, parse_naive_offset("+08:00"))))
        .collect();
    let at = |s: &str| Some(s.parse::<Timestamp>().unwrap());
    assert_eq!(
        got,
        vec![
            (
                "德國研究揭露AI開發平臺Lovable用戶開發的應用程式，有不少存在資料庫權限設定不當問題 - iThome".to_string(),
                "https://www.ithome.com.tw/news/180567".to_string(),
                at("2026-11-17T08:29:00Z"),
            ),
            (
                "Apple調整macOS App權限管理，27.3測試版開始可限制存取範圍 - iThome".to_string(),
                "https://www.ithome.com.tw/news/180559".to_string(),
                at("2026-11-17T04:59:00Z"),
            ),
            (
                "駭客利用AI自動化漏洞研究，遭到利用的零時差漏洞一個月爆增至22個 - iThome".to_string(),
                "https://www.ithome.com.tw/news/180561".to_string(),
                at("2026-11-17T04:32:00Z"),
            ),
        ]
    );
    assert_eq!(items[0].pub_date, "2026-11-17 16:29");
}

#[test]
fn ithome_fixture_without_an_offset_is_all_undated() {
    scratch_home();
    let body = include_bytes!("fixtures/ithome-ai-2026-11-17.xml");
    let (report, items) = process_body(&ithome(None), body, 30.0, ithome_now());
    assert_eq!(items, vec![]);
    assert_eq!(
        report,
        FeedReport {
            name: "iThome".into(),
            outcome: FeedOutcome::Ok,
            detail: None,
            counts: FeedCounts { fetched: 3, undated: 3, ..Default::default() },
        }
    );
}

#[test]
fn process_body_reads_the_first_date_with_the_feeds_offset() {
    // `published` is zoneless and `updated` is zoned. With the offset declared
    // the published date is the readable one and wins; it is 07:00Z here, and
    // the entry is kept with that date as written.
    scratch_home();
    let xml = br#"<feed xmlns="http://www.w3.org/2005/Atom">
      <entry><title>Lab note</title><link href="https://lab.example/1"/>
        <published>2026-11-17 15:00</published><updated>2026-11-17T08:40:00Z</updated></entry>
    </feed>"#;
    let spec = FeedSpec { naive_date_offset: Some("+08:00".into()), ..plain("Lab", 5) };
    let (report, items) = process_body(&spec, xml, 30.0, now());
    assert_eq!(report.counts, FeedCounts { fetched: 1, kept: 1, ..Default::default() });
    assert_eq!(items[0].pub_date, "2026-11-17 15:00");
}
