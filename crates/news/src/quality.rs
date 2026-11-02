//! Deciding whether an item is worth showing, before it becomes a bullet.
//!
//! Three verdicts. `Drop` removes the item (denied, promotional, or a
//! podcast/video page with no text to read); `TitleOnly` keeps its headline
//! because the body is behind a paywall and cannot be read; `Keep` is
//! everything else. The classification is deterministic — it does not change
//! depending on whether a network fetch happened to succeed — so a flaky
//! upstream cannot silently reshape the digest.

use regex::Regex;
use sha1::{Digest, Sha1};
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::sync::OnceLock;
use std::time::Duration;

/// Reputable but gated. Loaded and mergeable for forward compatibility; the
/// current deterministic classification does not consult it, and no accessor
/// exists until a branch actually gates on it.
const TRUSTED_SOURCES: [&str; 9] = [
    "Nikkei",
    "日經中文網",
    "Reuters",
    "Bloomberg",
    "Financial Times",
    "FT中文網",
    "中央社",
    "BBC",
    "The Wall Street Journal",
];

const PAYWALL_MARKERS: [&str; 10] = [
    "继续阅读",
    "繼續閱讀",
    "请登录",
    "請登入",
    "進入FT中文網",
    "进入FT中文网",
    "continue reading",
    "subscribe to read",
    "sign in to read",
    "已是会员",
];

/// Whole URL path segments that mark a page as audio or video, not text.
///
/// The digest links a reader to something to *read*. A podcast or video page
/// fetches as a script shell with no article text, so the model can only pick
/// it on its headline and the reader gets nothing behind the link (2026-10-31:
/// `radio.cgtn.com/podcast/news/5/...`, a 2.7 KB JS player). Each entry is
/// the bare section name publishers file such pages under:
///
/// - `podcast` / `podcasts` — audio episode pages (the cgtn case).
/// - `audio` — radio clips and audio-only explainers.
/// - `video` / `videos` — video players.
///
/// Intentionally narrow, and matched against whole segments only: a slug
/// like `video-china-rtx-5090-...` or `...-audio-102726` is a text article
/// *about* the medium, and both shapes appear in real decoded links. Every
/// false match drops real reporting, so a host that hides media elsewhere in
/// its path belongs on `deny_domains`, not in this list.
///
/// `watch` is left out on purpose. Its main target, YouTube's `/watch`, never
/// appeared among the publisher links the decode cache holds, while the same
/// word is an ordinary section name for text — wristwatch columns, "China
/// Watch"-style analysis series — so it would buy over-matching for no
/// observed gain.
const MEDIA_PATH_SEGMENTS: [&str; 5] = ["podcast", "podcasts", "audio", "video", "videos"];

/// Is this URL a non-text media page, judged from its path alone?
///
/// Deterministic by construction: no network and no body, so the answer is
/// the same whether or not a fetch would have succeeded. The query string and
/// fragment are not consulted. A string that does not parse as a URL is not
/// media.
pub fn url_is_media(url: &str) -> bool {
    let Some((_, path, _)) = crate::feed::split_url(url) else {
        return false;
    };
    path.split('/').any(|segment| {
        MEDIA_PATH_SEGMENTS
            .iter()
            .any(|m| segment.eq_ignore_ascii_case(m))
    })
}

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
(KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

fn config_path() -> std::path::PathBuf {
    crate::config::home().join(".nullclaw/news-quality-sources.json")
}

fn cache_root() -> std::path::PathBuf {
    crate::config::home().join(".nullclaw/.news-quality-cache")
}

/// Source names and hosts the operator has chosen to treat specially.
///
/// Every list starts empty on purpose. Shipping a publisher in a baked-in drop
/// list would silently remove a real source with no way to opt out, so the
/// operator owns them entirely via `~/.nullclaw/news-quality-sources.json`.
#[derive(Default, Debug, Clone)]
pub struct QualityConfig {
    pub trusted: HashSet<String>,
    /// Drop the item outright, by source name.
    pub deny: HashSet<String>,
    /// Drop the item outright, by host.
    pub deny_domains: HashSet<String>,
    /// Drop the item when the name shows up in the raw headline (Tier 1) or
    /// in the head of its post-precheck body excerpt — a person's byline,
    /// not an outlet name. Curated by the operator like `deny`.
    pub deny_authors: HashSet<String>,
    /// Keep the item, but only its headline.
    pub paywall_domains: HashSet<String>,
}

pub fn load_quality_config(path: Option<&std::path::Path>) -> QualityConfig {
    let mut cfg = QualityConfig {
        trusted: TRUSTED_SOURCES.iter().map(|s| s.to_string()).collect(),
        ..Default::default()
    };
    let owned = path.map(|p| p.to_path_buf()).unwrap_or_else(config_path);
    let Ok(text) = std::fs::read_to_string(&owned) else {
        return cfg;
    };
    let Ok(serde_json::Value::Object(data)) = serde_json::from_str::<serde_json::Value>(&text)
    else {
        return cfg;
    };
    let merge = |key: &str, into: &mut HashSet<String>| {
        if let Some(list) = data.get(key).and_then(|v| v.as_array()) {
            for v in list {
                match v {
                    serde_json::Value::String(s) => into.insert(s.clone()),
                    other => into.insert(other.to_string()),
                };
            }
        }
    };
    merge("trusted", &mut cfg.trusted);
    merge("deny", &mut cfg.deny);
    merge("deny_domains", &mut cfg.deny_domains);
    merge("deny_authors", &mut cfg.deny_authors);
    merge("paywall_domains", &mut cfg.paywall_domains);
    cfg
}

static ACTIVE: OnceLock<QualityConfig> = OnceLock::new();

pub fn active_config() -> &'static QualityConfig {
    ACTIVE.get_or_init(|| load_quality_config(None))
}

/// Host equals the domain, or is a subdomain of it.
///
/// Suffix-aware rather than substring: `ft.com` must not match `craft.com`.
pub fn host_in(host: &str, domains: &HashSet<String>) -> bool {
    if host.is_empty() {
        return false;
    }
    let lowered = host.to_lowercase();
    let host = lowered.trim_end_matches('.');
    domains.iter().any(|d| {
        let d = d.to_lowercase();
        let d = d.trim().trim_start_matches('.');
        !d.is_empty() && (host == d || host.ends_with(&format!(".{d}")))
    })
}

pub fn host_is_deny(host: &str) -> bool {
    host_in(host, &active_config().deny_domains)
}

pub fn host_is_paywall(host: &str) -> bool {
    host_in(host, &active_config().paywall_domains)
}

pub fn text_has_paywall_marker(text: &str) -> bool {
    PAYWALL_MARKERS.iter().any(|m| text.contains(m))
}

/// Anchored, unambiguous promotional markers only.
///
/// Bare substrings like 優惠／折扣／限時 are deliberately excluded: they
/// over-match ordinary business words inside legitimate news (限時降息) and
/// silently drop real stories. Promo dropping is title-only for the same
/// reason.
fn promo_patterns() -> &'static [Regex] {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        vec![
            Regex::new(r"(?i)\b\d+\s+(tips|ways|reasons|things)\b").expect("literal"),
            Regex::new(r"(?i)(press release|sponsored|advertorial)").expect("literal"),
            Regex::new(r"立即(选购|購買|下單)").expect("literal"),
        ]
    })
}

pub fn matches_promo_title(text: &str) -> bool {
    !text.is_empty() && promo_patterns().iter().any(|p| p.is_match(text))
}

// ── Google News URL decoding ─────────────────────────────────────────────────

/// Exposed for the differential probe; the payload and URL shapes must match
/// the Python byte for byte or Google rejects the request.
pub fn google_news_article_url_pub(rss_link: &str) -> String {
    google_news_article_url(rss_link)
}

pub fn build_batchexecute_payload_pub(id: &str, ts: &str, sg: &str) -> String {
    build_batchexecute_payload(id, ts, sg)
}

fn google_news_article_url(rss_link: &str) -> String {
    if rss_link.contains("hl=en-US") {
        return rss_link.to_string();
    }
    let sep = if rss_link.contains('?') { "&" } else { "?" };
    format!("{rss_link}{sep}hl=en-US&gl=US&ceid=US:en")
}

fn today_iso() -> String {
    jiff::Zoned::now().date().to_string()
}

fn cache_path(rss_link: &str) -> std::path::PathBuf {
    let digest = Sha1::digest(rss_link.as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    cache_root().join(today_iso()).join(format!("{hex}.url"))
}

fn cache_read(rss_link: &str) -> Option<String> {
    let text = std::fs::read_to_string(cache_path(rss_link)).ok()?;
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn cache_write(rss_link: &str, url: &str) {
    let path = cache_path(rss_link);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, url);
}

/// Drop per-day decode-cache directories older than the TTL, so it does not
/// grow one directory per day forever. Called alongside the skill's own sweep.
pub fn sweep_decode_cache(ttl_days: u64) {
    let Some(cutoff) =
        std::time::SystemTime::now().checked_sub(Duration::from_secs(ttl_days * 86400))
    else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(cache_root()) else {
        return;
    };
    for entry in entries.flatten() {
        let stale = entry
            .metadata()
            .ok()
            .filter(|m| m.is_dir())
            .and_then(|m| m.modified().ok())
            .is_some_and(|m| m < cutoff);
        if stale {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

fn attr_value(html: &str, attr: &str) -> Option<String> {
    let needle = format!("{attr}=\"");
    let start = html.find(&needle)? + needle.len();
    let rest = &html[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn extract_google_news_tokens(html: &str) -> Option<(String, String, String)> {
    Some((
        attr_value(html, "data-n-a-id")?,
        attr_value(html, "data-n-a-ts")?,
        attr_value(html, "data-n-a-sg")?,
    ))
}

/// Build the `f.req` body Google's `batchexecute` endpoint expects.
///
/// The nested array is opaque protocol shape, not data we choose — it is
/// reproduced exactly, placeholder `"X"` strings included, because the
/// endpoint rejects anything else.
fn build_batchexecute_payload(article_id: &str, article_ts: &str, article_sg: &str) -> String {
    use serde_json::{json, Value};
    let inner = serde_json::to_string(&json!([
        "garturlreq",
        [
            ["X", "X", ["X", "X"], Value::Null, Value::Null, 1, 1, "US:en",
             Value::Null, 1, Value::Null, Value::Null, Value::Null, Value::Null,
             Value::Null, 0, 1],
            "X",
            "X",
            1,
            [1, 1, 1],
            1,
            1,
            Value::Null,
            0,
            0,
            Value::Null,
            0
        ],
        article_id,
        article_ts,
        article_sg
    ]))
    .expect("serialisable");
    let inner = ensure_ascii(&inner);
    let outer = serde_json::to_string(&json!([[["Fbv4je", inner, Value::Null, "generic"]]]))
        .expect("serialisable");
    format!("f.req={}", quote_plus(&ensure_ascii(&outer)))
}

/// `json.dumps` defaults to `ensure_ascii=True`, so Python escapes every
/// non-ASCII codepoint as `\uXXXX` while serde writes it literally.
///
/// The article tokens are base64 and digits scraped from HTML attributes, so
/// today this changes nothing — which is exactly why it is worth pinning
/// rather than assuming. A payload byte Google does not expect is rejected
/// wholesale, and the failure would look like "decoding stopped working".
fn ensure_ascii(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c.is_ascii() {
            out.push(c);
        } else {
            let mut buf = [0u16; 2];
            for unit in c.encode_utf16(&mut buf) {
                out.push_str(&format!("\\u{unit:04x}"));
            }
        }
    }
    out
}

/// `urlencode`'s escaping: `quote_plus`, so a space becomes `+` and `/` is
/// escaped — unlike `quote`, which leaves `/` alone.
fn quote_plus(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        match b {
            b' ' => out.push('+'),
            b if b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-' | b'~') => {
                out.push(*b as char)
            }
            b => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn garturl_patterns() -> &'static [Regex] {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        vec![
            Regex::new(r#"garturlres\\?",\\"(https?://[^\\"]+)"#).expect("literal"),
            Regex::new(r#"garturlres","(https?://[^"]+)""#).expect("literal"),
            Regex::new(r#"garturlres[^h]*(https?://[^\s"\\]+)"#).expect("literal"),
        ]
    })
}

/// Three patterns in order, because the response shape varies with how deeply
/// the payload is escaped. The first that matches wins.
pub fn parse_garturlres(response_text: &str) -> Option<String> {
    let body = response_text.strip_prefix(")]}'").unwrap_or(response_text);
    garturl_patterns()
        .iter()
        .find_map(|p| p.captures(body))
        .map(|c| c[1].to_string())
}

fn http_get(url: &str, timeout: Duration) -> Result<(String, String), String> {
    let resp = claw_core::http::agent(timeout)
        .get(url)
        .set("User-Agent", USER_AGENT)
        .call()
        .map_err(|e| claw_core::http::error_class(&e))?;
    // The post-redirect URL, so the host used for classification reflects
    // where the request actually landed rather than where it was aimed.
    let final_url = resp.get_url().to_string();
    let mut body = String::new();
    resp.into_reader()
        .take(4 * 1024 * 1024)
        .read_to_string(&mut body)
        .map_err(|e| e.to_string())?;
    Ok((body, final_url))
}

fn http_post(url: &str, data: &str, timeout: Duration) -> Result<String, String> {
    claw_core::http::agent(timeout)
        .post(url)
        .set("User-Agent", USER_AGENT)
        .set(
            "Content-Type",
            "application/x-www-form-urlencoded;charset=UTF-8",
        )
        .send_string(data)
        .map_err(|e| claw_core::http::error_class(&e))?
        .into_string()
        .map_err(|e| e.to_string())
}

pub fn decode_google_news_url(rss_link: &str, timeout: Duration) -> Option<String> {
    if let Some(cached) = cache_read(rss_link) {
        return Some(cached);
    }
    let (html, _) = http_get(&google_news_article_url(rss_link), timeout).ok()?;
    let (id, ts, sg) = extract_google_news_tokens(&html)?;
    let payload = build_batchexecute_payload(&id, &ts, &sg);
    let response = http_post(
        "https://news.google.com/_/DotsSplashUi/data/batchexecute\
?rpcids=Fbv4je&source-path=%2Frss%2Farticles",
        &payload,
        timeout,
    )
    .ok()?;
    let decoded = parse_garturlres(&response)?;
    cache_write(rss_link, &decoded);
    Some(decoded)
}

// ── article body ─────────────────────────────────────────────────────────────

fn strip_html_patterns() -> &'static (Regex, Regex, Regex, Regex) {
    static P: OnceLock<(Regex, Regex, Regex, Regex)> = OnceLock::new();
    P.get_or_init(|| {
        (
            Regex::new(r"(?is)<script[^>]*>.*?</script>").expect("literal"),
            Regex::new(r"(?is)<style[^>]*>.*?</style>").expect("literal"),
            Regex::new(r"(?s)<[^>]+>").expect("literal"),
            Regex::new(r"\s+").expect("literal"),
        )
    })
}

pub fn strip_html(html: &str) -> String {
    let (script, style, tags, spaces) = strip_html_patterns();
    let t = script.replace_all(html, " ");
    let t = style.replace_all(&t, " ");
    let t = tags.replace_all(&t, " ");
    spaces.replace_all(&t, " ").trim().to_string()
}

#[derive(Debug, Clone, Default)]
pub struct Article {
    pub final_url: String,
    pub host: String,
    pub text: String,
    pub word_count: usize,
    pub truncated: bool,
    /// Set when the body could not be read at all.
    pub error: Option<String>,
}

pub fn fetch_article_text(url: &str, timeout: Duration) -> Article {
    match http_get(url, timeout) {
        Ok((html, final_url)) => {
            let text = strip_html(&html);
            let host = crate::feed::split_url(&final_url)
                .map(|(h, _, _)| h)
                .unwrap_or_default();
            let truncated = host_is_paywall(&host) || text_has_paywall_marker(&text);
            Article {
                word_count: text.split_whitespace().count(),
                final_url,
                host,
                text,
                truncated,
                error: None,
            }
        }
        Err(e) => Article {
            final_url: url.to_string(),
            error: Some(e),
            ..Default::default()
        },
    }
}

/// Could this fetched body top up this headline with a named entity?
///
/// Three rejection layers in order: no body; chrome or stub (word floor and
/// challenge markers — a Cloudflare interstitial strips to plausible prose
/// and passes the word floor alone); pairing (some significant Latin token of
/// the title must appear in the body). A title with no Latin token has no
/// pairing evidence, so it never gets an excerpt: enrichment only ever tops
/// up Latin entity names, and a CJK-bigram overlap would attach excerpts that
/// can never trigger a revision.
pub fn usable_excerpt(title: &str, article: &Article) -> Option<String> {
    if article.error.is_some() {
        return None;
    }
    let text = article.text.trim();
    if text.split_whitespace().count() < crate::config::excerpt_min_words() {
        return None;
    }
    let lower = text.to_lowercase();
    if CHALLENGE_MARKERS.iter().any(|m| lower.contains(m)) {
        return None;
    }
    let significant: HashSet<String> = crate::text::latin_tokens(title)
        .into_iter()
        .filter(|t| t.len() >= 4)
        .collect();
    if significant.is_empty() || !significant.iter().any(|t| lower.contains(t.as_str())) {
        return None;
    }
    let max = crate::config::excerpt_max_chars();
    let mut cut = text.len();
    for (i, ch) in text.char_indices() {
        if i + ch.len_utf8() > max {
            cut = i;
            break;
        }
    }
    Some(text[..cut].to_string())
}

/// Signatures a bot interstitial leaves in the stripped text.
const CHALLENGE_MARKERS: &[&str] = &[
    "just a moment",
    "enable javascript",
    "checking your browser",
    "attention required",
    "cf-chl",
    "verify you are a human",
];

// ── the verdict ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Keep,
    Drop,
    TitleOnly,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Verdict {
    pub action: Action,
    pub reason: Option<&'static str>,
    pub decoded_url: Option<String>,
    /// Leading window of the fetched body, gated by `usable_excerpt`,
    /// present only on `Keep`. Feeds the headline-enrichment prompt;
    /// classification itself never reads it.
    pub body_excerpt: Option<String>,
}

impl Verdict {
    fn new(
        action: Action,
        reason: Option<&'static str>,
        decoded_url: Option<String>,
        body_excerpt: Option<String>,
    ) -> Self {
        Self {
            action,
            reason,
            decoded_url,
            body_excerpt,
        }
    }
}

/// Classify from an item plus whatever body we managed to read.
pub fn classify_quality(
    source_name: &str,
    title: &str,
    article: Option<&Article>,
) -> (Action, Option<&'static str>) {
    let host = article.map(|a| a.host.as_str()).unwrap_or("");

    if active_config().deny.contains(source_name) || host_is_deny(host) {
        return (Action::Drop, Some("deny"));
    }
    // Title only, never body. The broad body patterns over-match ordinary
    // business words inside legitimate articles, and the LLM prompt already
    // excludes 純行銷推廣, so body-level promo dropping adds risk without gain.
    if matches_promo_title(title) {
        return (Action::Drop, Some("promo"));
    }
    // `precheck_action` asks the same question of the decoded URL before it
    // fetches; asking it again of the final URL catches a redirect that lands
    // on a player page. Before the paywall check: a paywalled player page
    // kept as `TitleOnly` would ship a headline over nothing to read — the
    // exact shape this rule exists to stop.
    if article.is_some_and(|a| url_is_media(&a.final_url)) {
        return (Action::Drop, Some("media"));
    }
    if host_is_paywall(host) || article.is_some_and(|a| a.truncated) {
        return (Action::TitleOnly, Some("paywalled"));
    }
    (Action::Keep, None)
}

/// Resolve one RSS item to a verdict. Never fails: every network step degrades
/// to a deterministic answer.
///
/// `cache` memoises decode-plus-fetch by link, so an item processed twice —
/// which the AI Level-3 re-subdivision does — touches the network once.
pub fn precheck_action(
    source_name: &str,
    title: &str,
    link: &str,
    decoded_url_hint: Option<&str>,
    decode_timeout: Duration,
    fetch_timeout: Duration,
    mut cache: Option<&mut HashMap<String, Verdict>>,
) -> Verdict {
    let hint = decoded_url_hint.filter(|s| !s.is_empty());

    if hint.is_none() {
        if let Some(c) = cache.as_deref() {
            if let Some(hit) = c.get(link) {
                return hit.clone();
            }
        }
    }

    fn finish(
        cache: &mut Option<&mut HashMap<String, Verdict>>,
        link: &str,
        v: Verdict,
    ) -> Verdict {
        if let Some(c) = cache.as_deref_mut() {
            if !link.is_empty() {
                c.insert(link.to_string(), v.clone());
            }
        }
        v
    }

    // Denying by source name needs no network at all.
    if active_config().deny.contains(source_name) {
        return finish(
            &mut cache,
            link,
            Verdict::new(Action::Drop, Some("deny"), None, None),
        );
    }

    let decoded_url = match hint.map(str::to_string) {
        Some(u) => Some(u),
        None => decode_google_news_url(link, decode_timeout),
    };
    let Some(decoded_url) = decoded_url else {
        // The publisher is unknown, so keep. Paywall and deny can only be
        // asserted once the host is known; an unknown host is left alone
        // rather than guessed at.
        return finish(
            &mut cache,
            link,
            Verdict::new(Action::Keep, Some("unresolved"), None, None),
        );
    };

    let host = crate::feed::split_url(&decoded_url)
        .map(|(h, _, _)| h)
        .unwrap_or_default();
    if host_is_deny(&host) {
        return finish(
            &mut cache,
            link,
            Verdict::new(Action::Drop, Some("deny"), Some(decoded_url), None),
        );
    }
    // A podcast or video page has no text to read, and the path says so
    // without a fetch, so it costs none. Ahead of the paywall check, for the
    // reason given in `classify_quality`: a paywalled player page must not
    // survive as a headline.
    if url_is_media(&decoded_url) {
        return finish(
            &mut cache,
            link,
            Verdict::new(Action::Drop, Some("media"), Some(decoded_url), None),
        );
    }
    if host_is_paywall(&host) {
        return finish(
            &mut cache,
            link,
            Verdict::new(Action::TitleOnly, Some("paywalled"), Some(decoded_url), None),
        );
    }

    let article = fetch_article_text(&decoded_url, fetch_timeout);
    if article.error.is_some() {
        // No body for a host that is neither denied nor paywalled. The body
        // promo check cannot run, so fall back to the title one.
        let v = if matches_promo_title(title) {
            Verdict::new(Action::Drop, Some("promo"), Some(decoded_url), None)
        } else {
            Verdict::new(Action::Keep, Some("fetch_error"), Some(decoded_url), None)
        };
        return finish(&mut cache, link, v);
    }

    let (action, reason) = classify_quality(source_name, title, Some(&article));
    let body_excerpt = if action == Action::Keep {
        usable_excerpt(title, &article)
    } else {
        None
    };
    finish(
        &mut cache,
        link,
        Verdict::new(action, reason, Some(decoded_url), body_excerpt),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The 2026-10-31 link that started this: a podcast player page.
    const CGTN_PODCAST: &str =
        "https://radio.cgtn.com/podcast/news/5/Beyond-DeepSeek-Whats-driving-Chinas-AI-surge/2722624";

    #[test]
    fn the_cgtn_podcast_page_is_media() {
        assert!(url_is_media(CGTN_PODCAST));
    }

    #[test]
    fn each_listed_segment_is_media_wherever_it_sits_in_the_path() {
        for url in [
            "https://example.com/podcasts/ep-12",
            "https://example.com/news/audio/clip-3",
            "https://example.com/world/video/2026/10/31/story",
            "https://example.com/videos/abc",
            "https://example.com/a/b/podcast",
        ] {
            assert!(url_is_media(url), "{url}");
        }
    }

    #[test]
    fn a_segment_is_matched_case_insensitively() {
        assert!(url_is_media("https://example.com/Podcast/ep-1"));
        assert!(url_is_media("https://example.com/news/VIDEO/x"));
        assert!(url_is_media("https://example.com/AuDiO/x"));
    }

    #[test]
    fn a_media_word_inside_a_longer_segment_is_not_media() {
        // The last four are real decoded publisher links from the decode
        // cache: articles *about* video and audio, filed under text slugs.
        for url in [
            "https://example.com/news/2026/10/videogame-ai-story",
            "https://example.com/tech/podcasting-trends",
            "https://futurism.com/artificial-intelligence/video-china-rtx-5090-graphics-cards-ai",
            "https://qz.com/google-deepmind-genie-4-world-model-audio-102726",
            "https://www.vaticannews.va/zht/pope/news/2026-10/pope-leo-xiv-videomessage-assisi-prayer-peace-interreligious.html",
            "https://www.orangenews.hk/culturevideo/VZiU9Un/story.shtml",
        ] {
            assert!(!url_is_media(url), "{url}");
        }
    }

    #[test]
    fn an_ordinary_article_is_not_media() {
        assert!(!url_is_media(
            "https://www.taipeitimes.com/News/taiwan/archives/2026/10/26/2003866622"
        ));
        assert!(!url_is_media("https://www.infoq.com/news/2026/10/google-meta-torchtpu/"));
    }

    #[test]
    fn the_query_and_fragment_are_not_read_as_path() {
        assert!(!url_is_media("https://example.com/story/1?type=video"));
        assert!(!url_is_media("https://example.com/story/1?next=/podcast/2"));
        assert!(!url_is_media("https://example.com/story/1#/video/"));
        assert!(!url_is_media("https://example.com?/audio/x"));
        // ...and they do not hide a media path either.
        assert!(url_is_media("https://example.com/video/1?utm_source=rss#top"));
    }

    #[test]
    fn a_string_that_is_not_a_url_is_not_media() {
        assert!(!url_is_media("/podcast/news/5"));
        assert!(!url_is_media(""));
    }

    #[test]
    fn the_shared_classifier_drops_a_body_whose_final_url_is_media() {
        // A link that redirected onto a player page reaches the classifier
        // with a fetched body; the path still decides. Not the cgtn host:
        // unit tests read the operator's real config through `$HOME`, and
        // that host is on its `deny_domains`, which would answer `deny` first.
        let article = Article {
            final_url: "https://media.example.com/podcast/news/5/beyond-deepseek/2722624"
                .to_string(),
            host: "media.example.com".to_string(),
            text: "player shell".to_string(),
            word_count: 2,
            ..Default::default()
        };
        assert_eq!(
            classify_quality("test-source", "Beyond DeepSeek?", Some(&article)),
            (Action::Drop, Some("media"))
        );
    }

    #[test]
    fn a_paywalled_player_page_is_dropped_not_kept_as_a_headline() {
        // `truncated` is the paywall signal that needs no operator config, so
        // the test does not depend on `$HOME`'s paywall_domains. If the
        // paywall check ran first this would be TitleOnly: a headline linking
        // to a player with nothing to read.
        let article = Article {
            final_url: "https://media.example.com/video/2026/10/31/briefing".to_string(),
            host: "media.example.com".to_string(),
            text: "subscribe to read".to_string(),
            word_count: 3,
            truncated: true,
            ..Default::default()
        };
        assert_eq!(
            classify_quality("test-source", "A briefing", Some(&article)),
            (Action::Drop, Some("media"))
        );
    }

    #[test]
    fn a_media_url_is_dropped_before_any_fetch() {
        // Port 1 on loopback refuses at once, so a fetch here would come back
        // as `fetch_error` (Keep). Only the pre-fetch path rule can answer
        // `media`, which is what proves no fetch was taken.
        let mut cache = HashMap::new();
        let v = precheck_action(
            "test-source",
            "Beyond DeepSeek: What's driving China's AI surge?",
            "http://127.0.0.1:1/rss-link",
            Some("http://127.0.0.1:1/podcast/news/5/beyond-deepseek/2722624"),
            Duration::from_millis(200),
            Duration::from_millis(200),
            Some(&mut cache),
        );
        assert_eq!(v.action, Action::Drop);
        assert_eq!(v.reason, Some("media"));
        assert_eq!(
            v.decoded_url.as_deref(),
            Some("http://127.0.0.1:1/podcast/news/5/beyond-deepseek/2722624")
        );
        assert_eq!(v.body_excerpt, None);
    }

    #[test]
    fn a_text_url_on_the_same_unreachable_host_does_reach_the_fetch() {
        // The control for the test above: same host, non-media path, so the
        // verdict comes from the failed fetch.
        let v = precheck_action(
            "test-source",
            "An ordinary headline",
            "http://127.0.0.1:1/rss-link-2",
            Some("http://127.0.0.1:1/news/2026/10/videogame-ai-story"),
            Duration::from_millis(200),
            Duration::from_millis(200),
            None,
        );
        assert_eq!((v.action, v.reason), (Action::Keep, Some("fetch_error")));
    }
}
