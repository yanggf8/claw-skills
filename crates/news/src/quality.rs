//! Deciding whether an item is worth showing, before it becomes a bullet.
//!
//! Three verdicts. `Drop` removes the item; `TitleOnly` keeps its headline
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
    let resp = ureq::builder()
        .timeout(timeout)
        .build()
        .get(url)
        .set("User-Agent", USER_AGENT)
        .call()
        .map_err(|e| error_class(&e))?;
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
    ureq::builder()
        .timeout(timeout)
        .build()
        .post(url)
        .set("User-Agent", USER_AGENT)
        .set(
            "Content-Type",
            "application/x-www-form-urlencoded;charset=UTF-8",
        )
        .send_string(data)
        .map_err(|e| error_class(&e))?
        .into_string()
        .map_err(|e| e.to_string())
}

/// Never the rendered error: `ureq`'s `Display` embeds the full URL.
fn error_class(e: &ureq::Error) -> String {
    match e {
        ureq::Error::Status(code, _) => format!("HTTP {code}"),
        ureq::Error::Transport(_) => "TransportError".to_string(),
    }
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
}

impl Verdict {
    fn new(action: Action, reason: Option<&'static str>, decoded_url: Option<String>) -> Self {
        Self {
            action,
            reason,
            decoded_url,
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
            Verdict::new(Action::Drop, Some("deny"), None),
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
            Verdict::new(Action::Keep, Some("unresolved"), None),
        );
    };

    let host = crate::feed::split_url(&decoded_url)
        .map(|(h, _, _)| h)
        .unwrap_or_default();
    if host_is_deny(&host) {
        return finish(
            &mut cache,
            link,
            Verdict::new(Action::Drop, Some("deny"), Some(decoded_url)),
        );
    }
    if host_is_paywall(&host) {
        return finish(
            &mut cache,
            link,
            Verdict::new(Action::TitleOnly, Some("paywalled"), Some(decoded_url)),
        );
    }

    let article = fetch_article_text(&decoded_url, fetch_timeout);
    if article.error.is_some() {
        // No body for a host that is neither denied nor paywalled. The body
        // promo check cannot run, so fall back to the title one.
        let v = if matches_promo_title(title) {
            Verdict::new(Action::Drop, Some("promo"), Some(decoded_url))
        } else {
            Verdict::new(Action::Keep, Some("fetch_error"), Some(decoded_url))
        };
        return finish(&mut cache, link, v);
    }

    let (action, reason) = classify_quality(source_name, title, Some(&article));
    finish(
        &mut cache,
        link,
        Verdict::new(action, reason, Some(decoded_url)),
    )
}
