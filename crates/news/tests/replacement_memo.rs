//! The paywall-replacement pass never reuses a memo verdict for a candidate,
//! even one whose link is its own decoded URL (every Bing candidate after
//! `normalize_replacement_candidate`). Two candidates can share a publisher
//! URL while carrying different titles and sources; each gets its own
//! precheck. Only the direct AI precheck may reuse a self-hinted verdict —
//! `tests/ai_sources.rs` pins that side.
//!
//! Offline: the search feeds are injected through
//! `resolve_paywall_replacements_with`, and the candidate's article is served
//! by a local stub that counts requests and fails closed (418).

use news::precheck::{
    resolve_paywall_replacements_with, same_story_gate, PaywallEntry, PaywallMap, StoryGate,
};
use news::text::Item;
use std::cell::RefCell;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

const ORIG: &str = "Nvidia Reports Record Quarterly Revenue - Reuters";
const CAND_A: &str = "Nvidia posts record revenue on AI demand - CNBC";
const CAND_B: &str = "Nvidia posts record quarterly revenue on AI chip demand - CNBC";

struct Home(PathBuf);

impl Home {
    fn new() -> Home {
        let home = std::env::temp_dir().join(format!("news-replacement-memo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join(".nullclaw")).unwrap();
        std::env::set_var("HOME", &home);
        std::env::set_var("NEWS_PRECHECK", "1");
        std::env::set_var("NEWS_PAYWALL_REPLACE", "1");
        std::env::set_var("NEWS_PAYWALL_REPLACE_SOURCES", "bing");
        std::env::set_var("NEWS_PAYWALL_REPLACE_DEADLINE", "20");
        std::env::set_var("NEWS_PAYWALL_REPLACE_MAX", "4");
        std::env::set_var("NEWS_PRECHECK_FETCH_TIMEOUT", "2");
        std::env::remove_var("NEWS_PAYWALL_REPLACE_BING_MKT");
        Home(home)
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// An article page that answers every request and reports each target.
fn article_stub() -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { break };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() {
                continue;
            }
            let target = line.split_whitespace().nth(1).unwrap_or("").to_string();
            loop {
                let mut h = String::new();
                match reader.read_line(&mut h) {
                    Ok(0) | Err(_) => break,
                    Ok(_) if h == "\r\n" => break,
                    Ok(_) => {}
                }
            }
            let (status, body) = if target == "/story" {
                (200, "<html><body><p>record revenue</p></body></html>")
            } else {
                (418, "unscripted")
            };
            let _ = tx.send(target);
            let mut out = stream;
            let _ = out.write_all(
                format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            );
        }
    });
    (base, rx)
}

fn drain(rx: &mpsc::Receiver<String>) -> Vec<String> {
    let mut out = Vec::new();
    while let Ok(t) = rx.recv_timeout(Duration::from_millis(200)) {
        out.push(t);
    }
    out
}

/// A Bing click-through, as the feed carries it.
fn bing_click(url: &str) -> String {
    let encoded = url.replace(':', "%3A").replace('/', "%2F");
    format!("https://www.bing.com/news/apiclick.aspx?ref=FexRss&url={encoded}")
}

#[test]
fn two_candidates_sharing_a_publisher_url_are_each_prechecked() {
    let _home = Home::new();
    let (base, seen) = article_stub();
    let story = format!("{base}/story");
    // Both pass the cheap gate, so both reach the precheck.
    assert_eq!(same_story_gate(ORIG, CAND_A), StoryGate::Same);
    assert_eq!(same_story_gate(ORIG, CAND_B), StoryGate::Same);

    let mut paywall: PaywallMap = PaywallMap::new();
    paywall.insert(
        1,
        PaywallEntry {
            decoded_url: Some("https://www.reuters.com/x".into()),
            reason: Some("paywalled".into()),
            title: ORIG.into(),
            source_name: "Reuters".into(),
            replacement: None,
            summary: None,
        },
    );

    let fetched_urls: RefCell<Vec<String>> = RefCell::new(Vec::new());
    let fetch = |url: &str, _max: usize, _t: Duration| -> Vec<Item> {
        fetched_urls.borrow_mut().push(url.to_string());
        if !url.contains("mkt=en-US") {
            return Vec::new();
        }
        [CAND_A, CAND_B]
            .iter()
            .map(|t| Item {
                title: t.to_string(),
                source: "CNBC".into(),
                link: bing_click(&story),
                ..Default::default()
            })
            .collect()
    };
    // The first candidate's headline cannot be translated, so the pass moves
    // on to the second — which must cost its own precheck.
    let translated: RefCell<Vec<String>> = RefCell::new(Vec::new());
    let translate = |title: &str, _date: &str| -> Option<String> {
        translated.borrow_mut().push(title.to_string());
        (title == CAND_B).then(|| "輝達季營收創新高".to_string())
    };
    let judge = |_: &str, _: &str, _: &str| -> Option<String> { None };

    resolve_paywall_replacements_with(&mut paywall, "2026/11/17 (Tue)", &translate, &judge, &fetch);

    // An English query goes to both Bing markets (`search_markets`).
    assert_eq!(
        *fetched_urls.borrow(),
        vec![
            "https://www.bing.com/news/search?q=Nvidia%20Reports%20Record%20Quarterly%20Revenue&mkt=en-US&format=rss",
            "https://www.bing.com/news/search?q=Nvidia%20Reports%20Record%20Quarterly%20Revenue&mkt=zh-TW&format=rss",
        ]
    );
    assert_eq!(*translated.borrow(), vec![CAND_A.to_string(), CAND_B.to_string()]);
    assert_eq!(drain(&seen), vec!["/story", "/story"]);
    let rep = paywall[&1].replacement.clone().expect("replacement");
    assert_eq!(rep.title_zh, "輝達季營收創新高");
    assert_eq!(rep.link, story);
    assert_eq!(rep.summary, "");
}
