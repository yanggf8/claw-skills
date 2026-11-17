//! Direct AI feeds, network half (spec §4 and §7's `ai_direct_feeds`): the
//! concurrent fetch, gzip, the total deadline, oversize bodies, per-feed
//! outcomes and the trace.
//!
//! Every request goes to a local stub. The stub fails closed: a path no test
//! scripted answers 418, which the fetch must report as `http_error`, so a
//! request the test did not expect shows up as a wrong outcome, not a pass.

use flate2::write::GzEncoder;
use flate2::Compression;
use jiff::Timestamp;
use news::direct::{
    fetch_direct_ai, process_body, DirectConfig, FeedCounts, FeedKind, FeedOutcome, FeedReport,
    FeedSpec, MAX_BODY_BYTES,
};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

fn scratch_home() -> &'static PathBuf {
    static HOME: OnceLock<PathBuf> = OnceLock::new();
    HOME.get_or_init(|| {
        let home = std::env::temp_dir().join(format!("news-direct-fetch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join(".nullclaw")).unwrap();
        std::env::set_var("HOME", &home);
        home
    })
}

#[derive(Clone)]
struct Route {
    status: u16,
    headers: Vec<(&'static str, String)>,
    body: Vec<u8>,
    delay: Duration,
}

fn ok_route(body: Vec<u8>) -> Route {
    Route { status: 200, headers: vec![], body, delay: Duration::ZERO }
}

struct Stub {
    base: String,
    /// (request path, User-Agent header) per request served.
    seen: mpsc::Receiver<(String, String)>,
}

impl Stub {
    fn start(routes: HashMap<&'static str, Route>) -> Stub {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let routes = Arc::new(routes);
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                let routes = Arc::clone(&routes);
                let tx = tx.clone();
                // One thread per connection, so a delayed route cannot hold
                // up the others — the fetch under test is concurrent.
                thread::spawn(move || serve(stream, &routes, &tx));
            }
        });
        Stub { base: format!("http://127.0.0.1:{port}"), seen: rx }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    fn seen(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        while let Ok(r) = self.seen.recv_timeout(Duration::from_millis(200)) {
            out.push(r);
        }
        out.sort();
        out
    }
}

fn serve(stream: TcpStream, routes: &HashMap<&'static str, Route>, tx: &mpsc::Sender<(String, String)>) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() {
        return;
    }
    let path = request_line.split_whitespace().nth(1).unwrap_or("").to_string();
    let mut ua = String::new();
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) if line == "\r\n" => break,
            Ok(_) => {
                if let Some((k, v)) = line.split_once(':') {
                    if k.eq_ignore_ascii_case("user-agent") {
                        ua = v.trim().to_string();
                    }
                }
            }
            Err(_) => break,
        }
    }
    let _ = tx.send((path.clone(), ua));
    let route = routes.get(path.as_str()).cloned().unwrap_or(Route {
        status: 418,
        headers: vec![],
        body: b"unscripted".to_vec(),
        delay: Duration::ZERO,
    });
    thread::sleep(route.delay);
    let mut out = stream;
    let mut head = format!(
        "HTTP/1.1 {} X\r\nContent-Length: {}\r\nConnection: close\r\n",
        route.status,
        route.body.len()
    );
    for (k, v) in &route.headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    let _ = out.write_all(head.as_bytes());
    let _ = out.write_all(&route.body);
    let _ = out.flush();
}

fn now() -> Timestamp {
    "2026-11-17T08:50:00Z".parse().unwrap()
}

fn rss(items: &[(&str, &str, &str)]) -> String {
    let mut s = String::from("<?xml version=\"1.0\"?><rss version=\"2.0\"><channel><title>t</title>");
    for (title, link, date) in items {
        s.push_str(&format!(
            "<item><title>{title}</title><link>{link}</link><pubDate>{date}</pubDate></item>"
        ));
    }
    s.push_str("</channel></rss>");
    s
}

fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut enc = GzEncoder::new(Vec::new(), Compression::default());
    enc.write_all(bytes).unwrap();
    enc.finish().unwrap()
}

fn spec(name: &str, url: String, max_items: usize) -> FeedSpec {
    FeedSpec { name: name.into(), url, kind: FeedKind::Plain, max_items, naive_date_offset: None }
}

fn config(feeds: Vec<FeedSpec>, deadline: f64) -> DirectConfig {
    DirectConfig { feeds, max_age_hours: 30.0, feeds_deadline_secs: deadline }
}

fn report(name: &str, outcome: FeedOutcome, detail: Option<&str>, counts: FeedCounts) -> FeedReport {
    FeedReport { name: name.into(), outcome, detail: detail.map(str::to_string), counts }
}

/// The `ai_direct_feeds` trace lines whose first feed is `first_feed` — tests
/// in this binary share one scratch trace file and run concurrently.
fn traced(first_feed: &str) -> Vec<serde_json::Value> {
    let path = scratch_home().join(".nullclaw/skill-traces.jsonl");
    let text = std::fs::read_to_string(path).unwrap_or_default();
    text.lines()
        .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
        .filter(|v| v["event"] == "ai_direct_feeds" && v["feeds"][0]["name"] == first_feed)
        .collect()
}

#[test]
fn gzip_encoded_feed_is_decoded_and_parsed() {
    scratch_home();
    let body = rss(&[
        ("Gemini 4 research note", "https://deepmind.google/blog/g4/", "Tue, 17 Nov 2026 06:00:00 GMT"),
        ("Older note", "https://deepmind.google/blog/old/", "Tue, 17 Nov 2026 05:00:00 GMT"),
    ]);
    let mut route = ok_route(gzip(body.as_bytes()));
    route.headers.push(("Content-Encoding", "gzip".into()));
    route.headers.push(("Content-Type", "application/rss+xml".into()));
    let stub = Stub::start(HashMap::from([("/deepmind.xml", route)]));

    let (items, rep) = fetch_direct_ai(
        &config(vec![spec("DeepMind", stub.url("/deepmind.xml"), 5)], 10.0),
        now(),
    );

    let titles: Vec<&str> = items.iter().map(|i| i.title.as_str()).collect();
    assert_eq!(titles, vec!["Gemini 4 research note - DeepMind", "Older note - DeepMind"]);
    assert_eq!(
        rep.feeds,
        vec![report("DeepMind", FeedOutcome::Ok, None, FeedCounts { fetched: 2, kept: 2, ..Default::default() })]
    );
    assert_eq!(rep.total_kept, 2);
    assert_eq!(
        stub.seen(),
        vec![("/deepmind.xml".to_string(), "nullclaw-news/1.0".to_string())]
    );
}

#[test]
fn deadline_records_a_pending_feed_as_timeout_and_keeps_the_rest() {
    scratch_home();
    let fresh = rss(&[("Fast story", "https://fast.example/1", "Tue, 17 Nov 2026 08:00:00 GMT")]);
    let mut slow = ok_route(fresh.clone().into_bytes());
    slow.delay = Duration::from_secs(6);
    let stub = Stub::start(HashMap::from([
        ("/fast.xml", ok_route(fresh.into_bytes())),
        ("/slow.xml", slow),
    ]));

    let started = Instant::now();
    let (items, rep) = fetch_direct_ai(
        &config(
            vec![
                spec("SlowFeed", stub.url("/slow.xml"), 5),
                spec("FastFeed", stub.url("/fast.xml"), 5),
            ],
            1.0,
        ),
        now(),
    );
    let took = started.elapsed();

    assert!(took < Duration::from_millis(2500), "fetch took {took:?}, deadline was 1s");
    assert_eq!(items.iter().map(|i| i.title.as_str()).collect::<Vec<_>>(), vec!["Fast story - FastFeed"]);
    assert_eq!(
        rep.feeds,
        vec![
            report("SlowFeed", FeedOutcome::Timeout, None, FeedCounts::default()),
            report("FastFeed", FeedOutcome::Ok, None, FeedCounts { fetched: 1, kept: 1, ..Default::default() }),
        ]
    );
    assert_eq!(rep.total_kept, 1);
}

#[test]
fn http_failures_are_http_error_with_the_status_class() {
    scratch_home();
    let empty_ok = Route { status: 204, headers: vec![], body: vec![], delay: Duration::ZERO };
    let gone = Route { status: 404, headers: vec![], body: b"nope".to_vec(), delay: Duration::ZERO };
    let stub = Stub::start(HashMap::from([("/gone.xml", gone), ("/empty.xml", empty_ok)]));
    // A port that was bound and released: nothing listens there.
    let refused = {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        format!("http://127.0.0.1:{}/feed.xml", l.local_addr().unwrap().port())
    };

    let (items, rep) = fetch_direct_ai(
        &config(
            vec![
                spec("Gone", stub.url("/gone.xml"), 5),
                spec("NoContent", stub.url("/empty.xml"), 5),
                spec("Refused", refused, 5),
                spec("Unscripted", stub.url("/nobody-scripted-this"), 5),
            ],
            10.0,
        ),
        now(),
    );

    assert_eq!(items, vec![]);
    assert_eq!(
        rep.feeds,
        vec![
            report("Gone", FeedOutcome::HttpError, Some("HTTP 404"), FeedCounts::default()),
            report("NoContent", FeedOutcome::HttpError, Some("HTTP 204"), FeedCounts::default()),
            report("Refused", FeedOutcome::HttpError, Some("transport error"), FeedCounts::default()),
            report("Unscripted", FeedOutcome::HttpError, Some("HTTP 418"), FeedCounts::default()),
        ]
    );
    assert_eq!(rep.total_kept, 0);
}

/// A well-formed feed padded with an XML comment to exactly `len` bytes.
fn padded_feed(len: usize) -> Vec<u8> {
    let head = "<rss><channel><item><title>Big</title><link>https://big.example/1</link><pubDate>Tue, 17 Nov 2026 08:00:00 GMT</pubDate></item><!--";
    let tail = "--></channel></rss>";
    let mut s = String::from(head);
    s.push_str(&"x".repeat(len - head.len() - tail.len()));
    s.push_str(tail);
    assert_eq!(s.len(), len);
    s.into_bytes()
}

#[test]
fn oversize_body_is_rejected_before_parsing_and_the_limit_itself_passes() {
    scratch_home();
    assert_eq!(MAX_BODY_BYTES, 5 * 1024 * 1024);
    let stub = Stub::start(HashMap::from([
        ("/at-limit.xml", ok_route(padded_feed(MAX_BODY_BYTES))),
        ("/over.xml", ok_route(padded_feed(MAX_BODY_BYTES + 1))),
    ]));

    let (items, rep) = fetch_direct_ai(
        &config(
            vec![
                spec("AtLimit", stub.url("/at-limit.xml"), 5),
                spec("Over", stub.url("/over.xml"), 5),
            ],
            20.0,
        ),
        now(),
    );

    assert_eq!(items.iter().map(|i| i.title.as_str()).collect::<Vec<_>>(), vec!["Big - AtLimit"]);
    assert_eq!(
        rep.feeds,
        vec![
            report("AtLimit", FeedOutcome::Ok, None, FeedCounts { fetched: 1, kept: 1, ..Default::default() }),
            report("Over", FeedOutcome::Oversize, Some("body exceeds 5242880 bytes"), FeedCounts::default()),
        ]
    );
}

#[test]
fn process_body_rejects_oversize_without_parsing() {
    scratch_home();
    let (rep, items) = process_body(
        &spec("Direct", "https://d.example/rss".into(), 5),
        &padded_feed(MAX_BODY_BYTES + 1),
        30.0,
        now(),
    );
    assert_eq!(items, vec![]);
    assert_eq!(
        rep,
        report("Direct", FeedOutcome::Oversize, Some("body exceeds 5242880 bytes"), FeedCounts::default())
    );
}

#[test]
fn html_body_is_parse_error() {
    scratch_home();
    let stub = Stub::start(HashMap::from([(
        "/html.xml",
        ok_route(b"<html><body>Our systems have detected unusual traffic</body></html>".to_vec()),
    )]));
    let (_, rep) = fetch_direct_ai(&config(vec![spec("Html", stub.url("/html.xml"), 5)], 10.0), now());
    assert_eq!(
        rep.feeds,
        vec![report(
            "Html",
            FeedOutcome::ParseError,
            Some("root element <html> is not rss, feed or RDF"),
            FeedCounts::default(),
        )]
    );
}

#[test]
fn items_concatenate_in_config_order_and_the_trace_matches_the_report() {
    scratch_home();
    let a = rss(&[
        ("A old", "https://a.example/1", "Tue, 17 Nov 2026 01:00:00 GMT"),
        ("A stale", "https://a.example/2", "Mon, 02 Jan 2023 10:00:00 GMT"),
        ("A new", "https://a.example/3", "Tue, 17 Nov 2026 08:00:00 GMT"),
    ]);
    let b = rss(&[
        ("B undated", "https://b.example/1", ""),
        ("B badlink", "javascript:void(0)", "Tue, 17 Nov 2026 07:00:00 GMT"),
        ("B ok", "https://b.example/3", "Tue, 17 Nov 2026 02:00:00 GMT"),
    ]);
    let stub = Stub::start(HashMap::from([
        ("/a.xml", ok_route(a.into_bytes())),
        ("/b.xml", ok_route(b.into_bytes())),
        ("/c.xml", Route { status: 500, headers: vec![], body: vec![], delay: Duration::ZERO }),
    ]));

    let (items, rep) = fetch_direct_ai(
        &config(
            vec![
                spec("TraceA", stub.url("/a.xml"), 5),
                spec("TraceB", stub.url("/b.xml"), 5),
                spec("TraceC", stub.url("/c.xml"), 5),
            ],
            10.0,
        ),
        now(),
    );

    assert_eq!(
        items.iter().map(|i| i.title.as_str()).collect::<Vec<_>>(),
        vec!["A new - TraceA", "A old - TraceA", "B ok - TraceB"]
    );
    assert_eq!(rep.total_kept, 3);

    let lines = traced("TraceA");
    assert_eq!(lines.len(), 1, "exactly one ai_direct_feeds event per build");
    let line = &lines[0];
    assert_eq!(line["skill"], "news");
    assert_eq!(
        line["feeds"],
        serde_json::json!([
            {"name": "TraceA", "outcome": "ok", "detail": null, "fetched": 3, "malformed": 0,
             "stale": 1, "undated": 0, "future": 0, "invalid_link": 0, "capped": 0, "kept": 2},
            {"name": "TraceB", "outcome": "ok", "detail": null, "fetched": 3, "malformed": 0,
             "stale": 0, "undated": 1, "future": 0, "invalid_link": 1, "capped": 0, "kept": 1},
            {"name": "TraceC", "outcome": "http_error", "detail": "HTTP 500", "fetched": 0,
             "malformed": 0, "stale": 0, "undated": 0, "future": 0, "invalid_link": 0, "capped": 0,
             "kept": 0},
        ])
    );
    assert_eq!(line["total_kept"], 3);
    assert_eq!(line["elapsed_ms"], serde_json::json!(rep.elapsed_ms));
    assert_eq!(rep.to_json()["feeds"], line["feeds"]);
}

#[test]
fn truncated_body_is_parse_error_not_ok() {
    scratch_home();
    let (rep, items) = process_body(
        &spec("Cut", "https://c.example/rss".into(), 5),
        b"<rss><channel><item><title>A</title><link>https://c.example/1</link><pubDate>Tue, 17 Nov 2026 08:00:00 GMT</pubDate></item>",
        30.0,
        now(),
    );
    assert_eq!(items, vec![]);
    assert_eq!(
        rep,
        report(
            "Cut",
            FeedOutcome::ParseError,
            Some("truncated: 2 element(s) still open at end of input"),
            FeedCounts::default(),
        )
    );
}

#[test]
fn the_size_limit_applies_to_decompressed_bytes() {
    scratch_home();
    let over = gzip(&padded_feed(MAX_BODY_BYTES + 1));
    let at = gzip(&padded_feed(MAX_BODY_BYTES));
    // On the wire both are tiny; only the decoded size can trip the limit.
    assert!(over.len() < MAX_BODY_BYTES / 100, "wire size {}", over.len());
    let gz = |body: Vec<u8>| {
        let mut r = ok_route(body);
        r.headers.push(("Content-Encoding", "gzip".into()));
        r
    };
    let stub = Stub::start(HashMap::from([("/gz-over.xml", gz(over)), ("/gz-at.xml", gz(at))]));

    let (items, rep) = fetch_direct_ai(
        &config(
            vec![
                spec("GzOver", stub.url("/gz-over.xml"), 5),
                spec("GzAt", stub.url("/gz-at.xml"), 5),
            ],
            20.0,
        ),
        now(),
    );

    assert_eq!(items.iter().map(|i| i.title.as_str()).collect::<Vec<_>>(), vec!["Big - GzAt"]);
    assert_eq!(
        rep.feeds,
        vec![
            report("GzOver", FeedOutcome::Oversize, Some("body exceeds 5242880 bytes"), FeedCounts::default()),
            report("GzAt", FeedOutcome::Ok, None, FeedCounts { fetched: 1, kept: 1, ..Default::default() }),
        ]
    );
}
