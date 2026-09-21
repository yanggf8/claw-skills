//! The week's raw material: eight Google News searches, deduplicated.
//!
//! These are the only thing the skill itself contributes to the article. Voice,
//! history, editorial plan, body storage and delivery all live in
//! `persona-core`; this module decides what the writer gets to look at.

use claw_core::http::{agent, error_class};
use quick_xml::events::Event;
use quick_xml::Reader;
use std::io::Write;
use std::time::Duration;

/// Five English and three Chinese. Not a deliberate ratio so much as what has
/// accumulated — worth knowing when reading a week where the material skews
/// anglophone, since that is the feed and not the topic.
pub const QUERIES: [&str; 8] = [
    "mindfulness AI",
    "meditation technology",
    "AI spirituality",
    "冥想 AI",
    "正念 數位",
    "身心靈 科技",
    "AI consciousness",
    "artificial intelligence philosophy",
];

pub const PER_QUERY: usize = 5;
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(10);
const USER_AGENT: &str = "Mozilla/5.0";

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub id: usize,
    pub title: String,
    pub url: String,
    pub source: String,
}

/// A query with any CJK character is treated as Chinese and gets the Taiwan
/// locale; everything else gets US English. Mixing them would return the same
/// English wire copy for both halves of the query list.
pub fn is_chinese(query: &str) -> bool {
    query.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c))
}

pub fn search_url(query: &str) -> String {
    let (hl, gl, ceid) = if is_chinese(query) {
        ("zh-TW", "TW", "TW:zh-Hant")
    } else {
        ("en-US", "US", "US:en")
    };
    format!(
        "https://news.google.com/rss/search?q={}&hl={hl}&gl={gl}&ceid={ceid}",
        quote(query)
    )
}

/// Percent-encoding as `urllib.parse.quote` does it: the unreserved set plus
/// `/`, which it leaves alone by default.
pub fn quote(s: &str) -> String {
    const KEEP: &[u8] = b"_.-~/";
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        if b.is_ascii_alphanumeric() || KEEP.contains(b) {
            out.push(*b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// `(title, link, source)` for up to [`PER_QUERY`] items.
///
/// Scoped to `<root>/channel/item` and its *direct* children, which is what
/// the Python's `./channel/item` and `item.find("title")` mean. An `<item>`
/// nested somewhere else, or a `<title>` belonging to a child element, cannot
/// become material. An entry missing either a title or a link is skipped
/// rather than half-used.
pub fn parse_feed(xml: &str, limit: usize) -> Vec<(String, String, String)> {
    /// root / channel / item
    const ITEM_DEPTH: usize = 3;

    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);

    let mut out = Vec::new();
    let mut path: Vec<String> = Vec::new();
    let mut field: Option<&'static str> = None;
    let (mut title, mut link, mut source) = (None, None, None);

    let at_item = |p: &[String]| p.len() == ITEM_DEPTH && p[1] == "channel" && p[2] == "item";

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                path.push(local_name(e.name().as_ref()));
                if at_item(&path) {
                    title = None;
                    link = None;
                    source = None;
                } else if path.len() == ITEM_DEPTH + 1 && at_item(&path[..ITEM_DEPTH]) {
                    field = match path[ITEM_DEPTH].as_str() {
                        "title" if title.is_none() => Some("title"),
                        "link" if link.is_none() => Some("link"),
                        "source" if source.is_none() => Some("source"),
                        _ => None,
                    };
                    match field {
                        Some("title") => title = Some(String::new()),
                        Some("link") => link = Some(String::new()),
                        Some("source") => source = Some(String::new()),
                        _ => {}
                    }
                }
            }
            Ok(Event::Text(t)) if field.is_some() => {
                let s = t.unescape().unwrap_or_default().into_owned();
                push(field, &mut title, &mut link, &mut source, &s);
            }
            Ok(Event::CData(c)) if field.is_some() => {
                let s = String::from_utf8_lossy(&c.into_inner()).into_owned();
                push(field, &mut title, &mut link, &mut source, &s);
            }
            Ok(Event::End(_)) => {
                let closing_item = at_item(&path);
                if closing_item {
                    let t = title.take().unwrap_or_default();
                    let l = link.take().unwrap_or_default();
                    if !t.is_empty() && !l.is_empty() {
                        // `<source>` is optional; Google News supplies it, and
                        // the fallback keeps the citation line renderable when
                        // some other feed does not.
                        let s = source
                            .take()
                            .filter(|s| !s.is_empty())
                            .unwrap_or_else(|| "Google News".to_string());
                        out.push((t, l, s));
                        if out.len() >= limit {
                            break;
                        }
                    }
                    source = None;
                }
                field = None;
                path.pop();
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    out
}

fn push(
    field: Option<&'static str>,
    title: &mut Option<String>,
    link: &mut Option<String>,
    source: &mut Option<String>,
    text: &str,
) {
    let slot = match field {
        Some("title") => title,
        Some("link") => link,
        _ => source,
    };
    if let Some(buf) = slot.as_mut() {
        buf.push_str(text);
    }
}

fn local_name(raw: &[u8]) -> String {
    let s = String::from_utf8_lossy(raw);
    match s.rfind(':') {
        Some(i) => s[i + 1..].to_string(),
        None => s.into_owned(),
    }
}

/// One query's results. A failed fetch is a warning and an empty list: eight
/// searches feed this, and one dead query must not cost the week's article.
pub fn search(query: &str, base: Option<&str>, err: &mut impl Write) -> Vec<(String, String, String)> {
    let url = match base {
        Some(b) => format!("{b}/rss/search?q={}", quote(query)),
        None => search_url(query),
    };
    match agent(FETCH_TIMEOUT)
        .get(&url)
        .set("User-Agent", USER_AGENT)
        .call()
        .map_err(|e| error_class(&e))
        .and_then(|r| r.into_string().map_err(|e| e.to_string()))
    {
        Ok(body) => parse_feed(&body, PER_QUERY),
        Err(class) => {
            let _ = writeln!(err, "Error fetching RSS for {query}: {class}");
            Vec::new()
        }
    }
}

/// Every query's results, deduplicated by URL, numbered from one.
///
/// The number is what the writer cites as `[來源 #N]` and what persona-core
/// later restores into real links, so it has to be assigned once, here, and
/// stay stable for the rest of the run.
pub fn collect(base: Option<&str>, err: &mut impl Write) -> Vec<Item> {
    let mut items: Vec<Item> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for query in QUERIES {
        for (title, url, source) in search(query, base, err) {
            if !seen.insert(url.clone()) {
                continue;
            }
            items.push(Item {
                id: items.len() + 1,
                title,
                url,
                source,
            });
        }
    }
    items
}

/// The numbered list the writer sees in its prompt.
pub fn prompt_items(items: &[Item]) -> String {
    items
        .iter()
        .map(|i| format!("#{} [{}] {}", i.id, i.source, i.title))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The TSV persona-core reads to turn `[來源 #N]` back into links.
pub fn material_text(items: &[Item]) -> String {
    let body = items
        .iter()
        .map(|i| format!("{}\t{}\t{}", i.id, i.source, i.url))
        .collect::<Vec<_>>()
        .join("\n");
    format!("{body}\n")
}
