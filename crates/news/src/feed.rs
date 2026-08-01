//! Fetching and parsing RSS.
//!
//! Two upstreams: Google News (the default feeds and per-topic searches) and
//! Bing (only ever consulted for a free replacement of a paywalled pick).

use crate::config::paywall_replace_bing_mkt;
use crate::text::{extract_source_name, Item};
use quick_xml::events::Event;
use quick_xml::Reader;
use std::time::Duration;

/// The built-in feeds, in the order the digest reads them.
pub const FEEDS: [(&str, &str); 6] = [
    // AI — broad US coverage: research, policy, industry.
    ("ai_us", "https://news.google.com/rss/search?q=artificial+intelligence+AI+breakthrough+OR+regulation+OR+research+when:1d&hl=en-US&gl=US&ceid=US:en"),
    // AI — the major labs and their products.
    ("ai_labs", "https://news.google.com/rss/search?q=OpenAI+OR+Anthropic+OR+Google+DeepMind+OR+Meta+AI+OR+xAI+when:1d&hl=en-US&gl=US&ceid=US:en"),
    // AI — China, in English coverage.
    ("ai_cn", "https://news.google.com/rss/search?q=China+AI+OR+Baidu+AI+OR+DeepSeek+OR+Alibaba+AI+OR+ByteDance+AI+when:1d&hl=en-US&gl=US&ceid=US:en"),
    // AI — Taiwan local.
    ("ai_tw", "https://news.google.com/rss/search?q=AI+when:1d&hl=zh-TW&gl=TW&ceid=TW:zh-Hant"),
    ("tech", "https://news.google.com/rss/search?q=%E7%A7%91%E6%8A%80+%E5%8D%8A%E5%B0%8E%E9%AB%94+%E6%99%B6%E7%89%87+when:1d&hl=zh-TW&gl=TW&ceid=TW:zh-Hant"),
    ("general", "https://news.google.com/rss?hl=zh-TW&gl=TW&ceid=TW:zh-Hant"),
];

pub fn feed_url(name: &str) -> Option<&'static str> {
    FEEDS.iter().find(|(k, _)| *k == name).map(|(_, v)| *v)
}

/// Percent-encode for a query value, matching Python's `urllib.parse.quote`.
///
/// `quote` keeps `/` unescaped by default alongside the unreserved set. That
/// matters: a topic containing a slash must encode the same way in both
/// implementations or the two hit different Google URLs.
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

pub fn topic_feed_url(topic: &str) -> String {
    format!(
        "https://news.google.com/rss/search?q={}+when:1d&hl=zh-TW&gl=TW&ceid=TW:zh-Hant",
        quote(topic)
    )
}

pub fn bing_news_feed_url(query: &str) -> String {
    format!(
        "https://www.bing.com/news/search?q={}&mkt={}&format=rss",
        quote(query),
        paywall_replace_bing_mkt()
    )
}

/// Fetch a feed and parse its items. An upstream that is slow, down, or
/// answering rubbish yields no items — never an error the caller must handle,
/// because one dead feed must not stop a digest built from six.
pub fn fetch_feed(url: &str, max_items: usize, timeout: Duration) -> Vec<Item> {
    // The error is reduced to a class immediately rather than carried, both
    // to keep the URL out of the diagnostic and because a `ureq::Error` owns a
    // whole response.
    let fetched = ureq::builder()
        .timeout(timeout)
        .build()
        .get(url)
        .set("User-Agent", "nullclaw-news/1.0")
        .call()
        .map_err(|e| fetch_error(&e))
        .and_then(|r| r.into_string().map_err(|e| e.to_string()));

    let body = match fetched {
        Ok(b) => b,
        Err(class) => {
            // The URL is truncated the same way the Python truncates it. These
            // are search URLs with no credentials in them, but the habit is
            // worth keeping: a fetch diagnostic should not be a place secrets
            // can appear.
            let head: String = url.chars().take(60).collect();
            eprintln!("[WARN] fetch failed: {head}... {class}");
            return Vec::new();
        }
    };
    parse_rss(&body, max_items)
}

/// The status or transport class, never the rendered error.
///
/// `ureq`'s `Display` includes the full URL with its query string.
fn fetch_error(e: &ureq::Error) -> String {
    match e {
        ureq::Error::Status(code, _) => format!("HTTP {code}"),
        ureq::Error::Transport(_) => "transport error".to_string(),
    }
}

/// Pull `title` / `link` / `pubDate` out of each `<item>`.
///
/// Only the first occurrence of each child is taken and only its direct text,
/// which is what Python's `findtext` does. A malformed document yields
/// whatever was parsed before the break, matching `ET.ParseError` being
/// swallowed after the loop has already appended.
pub fn parse_rss(xml: &str, max_items: usize) -> Vec<Item> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);

    let mut items: Vec<Item> = Vec::new();
    let mut in_item = false;
    let mut field: Option<&'static str> = None;
    let (mut title, mut link, mut pub_date) = (None, None, None);

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let name = e.name();
                let tag = local_name(name.as_ref());
                if tag == b"item" {
                    in_item = true;
                    title = None;
                    link = None;
                    pub_date = None;
                } else if in_item {
                    field = match tag {
                        b"title" if title.is_none() => Some("title"),
                        b"link" if link.is_none() => Some("link"),
                        b"pubDate" if pub_date.is_none() => Some("pubDate"),
                        _ => None,
                    };
                    if let Some(f) = field {
                        match f {
                            "title" => title = Some(String::new()),
                            "link" => link = Some(String::new()),
                            _ => pub_date = Some(String::new()),
                        }
                    }
                }
            }
            Ok(Event::Text(t)) if field.is_some() => {
                // Entities are resolved here; `&amp;` in a headline must reach
                // the reader as `&`.
                let decoded = t.unescape().unwrap_or_default().into_owned();
                push_field(field, &mut title, &mut link, &mut pub_date, &decoded);
            }
            Ok(Event::CData(c)) if field.is_some() => {
                // CDATA is literal by definition — unescaping it would corrupt
                // a headline that legitimately contains `&amp;`.
                let raw = String::from_utf8_lossy(&c.into_inner()).into_owned();
                push_field(field, &mut title, &mut link, &mut pub_date, &raw);
            }
            Ok(Event::End(e)) => {
                let name = e.name();
                let tag = local_name(name.as_ref());
                if tag == b"item" {
                    in_item = false;
                    field = None;
                    let t = title.take().unwrap_or_default().trim().to_string();
                    if !t.is_empty() {
                        items.push(Item {
                            source: extract_source_name(&t),
                            title: t,
                            link: link.take().unwrap_or_default().trim().to_string(),
                            pub_date: pub_date.take().unwrap_or_default().trim().to_string(),
                            decoded_url: None,
                        });
                        if items.len() >= max_items {
                            break;
                        }
                    }
                } else if field.is_some() {
                    field = None;
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    items
}

fn push_field(
    field: Option<&'static str>,
    title: &mut Option<String>,
    link: &mut Option<String>,
    pub_date: &mut Option<String>,
    text: &str,
) {
    let slot = match field {
        Some("title") => title,
        Some("link") => link,
        _ => pub_date,
    };
    if let Some(buf) = slot.as_mut() {
        buf.push_str(text);
    }
}

/// Strip any namespace prefix, so `<dc:title>` and `<title>` compare alike —
/// `ElementTree` would keep them distinct via a `{uri}` prefix, but no RSS in
/// use here namespaces the three fields read above.
fn local_name(raw: &[u8]) -> &[u8] {
    match raw.iter().rposition(|b| *b == b':') {
        Some(i) => &raw[i + 1..],
        None => raw,
    }
}

/// Bing wraps result links in a click tracker; unwrap it to the real article.
///
/// Left alone, every replacement link would point at bing.com, and the
/// same-registered-domain check that stops a paywalled source replacing itself
/// would compare the wrong host.
pub fn normalize_replacement_candidate(mut item: Item) -> Item {
    let Some((host, path, query)) = split_url(&item.link) else {
        return item;
    };
    if !host.to_lowercase().ends_with("bing.com") || !path.ends_with("/news/apiclick.aspx") {
        return item;
    }
    if let Some(direct) = query_param(query, "url").filter(|d| !d.is_empty()) {
        item.decoded_url = Some(direct.clone());
        item.link = direct;
    }
    item
}

/// `(netloc, path, query)`. Deliberately minimal — enough for the two checks
/// that need it, with no dependency on a URL crate.
///
/// The first element is the *whole* authority, userinfo and port included,
/// because that is what `urlparse().netloc` gives Python and what the
/// paywall/deny host matching has always compared against. Stripping them
/// would look like a tidy-up and quietly change classification: `ft.com:8443`
/// does not currently match a `ft.com` paywall entry, and it must keep not
/// matching until someone decides otherwise on purpose.
pub fn split_url(url: &str) -> Option<(String, String, &str)> {
    let rest = url.split_once("://")?.1;
    let (netloc, tail) = match rest.find(['/', '?', '#']) {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    let (path, query) = match tail.find('?') {
        Some(i) => (&tail[..i], &tail[i + 1..]),
        None => (tail, ""),
    };
    let path = path.split('#').next().unwrap_or(path);
    let query = query.split('#').next().unwrap_or(query);
    Some((netloc.to_string(), path.to_string(), query))
}

/// First value for `name`, percent-decoded, matching `parse_qs`'s `[0]`.
pub fn query_param(query: &str, name: &str) -> Option<String> {
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| percent_decode(v))
}

/// `parse_qs` treats `+` as a space, which matters for a wrapped URL's query.
pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
                match hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                    Some(b) => {
                        out.push(b);
                        i += 3;
                    }
                    None => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}
