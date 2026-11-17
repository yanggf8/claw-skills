//! Direct AI publisher feeds — ingestion only.
//!
//! Spec: `docs/specs/2026-11-17-news-ai-direct-feeds-design.md` §1–§4 and the
//! `ai_direct_feeds` trace of §7. This module turns the operator's feed list
//! into Google-shaped [`Item`]s plus a per-feed report. It decides nothing
//! about delivery: whether an empty result alerts, how the AI section uses
//! the items and which cache variant they live under belong to the caller.
//!
//! Every item leaves here satisfying the invariant the rest of the digest
//! assumes of a Google item — `title == "{headline} - {source}"` — so
//! `extract_source_name`, `title_without_source` and every rule keyed by
//! source name work on it unchanged.

use crate::config::ai_feeds_file;
use crate::feed::{parse_feed_all_at, parse_feed_date_at, split_url, FeedEntry, USER_AGENT};
use crate::precheck::publisher_link;
use crate::text::Item;
use crate::trace::log_trace;
use jiff::Timestamp;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

/// A body larger than this is rejected as `oversize` before it is parsed.
/// Archives are parsed whole (§3), so the response size is the only bound on
/// that work.
pub const MAX_BODY_BYTES: usize = 5 * 1024 * 1024;

/// One feed request's own bound — the same 15 s `main.rs` gives a Google
/// feed. Independent of `feeds_deadline_secs` on purpose: the deadline bounds
/// how long the caller waits, this bounds how long an abandoned worker lives.
pub const FEED_TIMEOUT: Duration = Duration::from_secs(15);

/// How far ahead of `now` an entry's date may be and still count as current.
///
/// A publisher's clock or timezone can be a little off, so an entry up to
/// this far ahead is kept — but sorted as if published `now`, so it cannot
/// jump ahead of genuinely current entries. Beyond it the date is wrong, not
/// skewed: such an entry is dropped as `future`. Left unchecked, a feed with a
/// broken clock would sort its whole archive first, fill its `max_items`, and
/// — because feeds are concatenated in config order and the section takes the
/// first N — could crowd the entire AI candidate list.
pub const FUTURE_TOLERANCE: Duration = Duration::from_secs(3600);

// ── §1 config ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FeedKind {
    /// RSS or Atom; the feed's `name` is every item's source.
    #[default]
    Plain,
    /// Techmeme's aggregator shape: the source and the article link are
    /// unwrapped from each item (§2).
    Techmeme,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeedSpec {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub kind: FeedKind,
    /// Applied per feed **after** filtering.
    pub max_items: usize,
    /// `±HH:MM`: the clock of a feed whose dates carry no zone at all
    /// (iThome's `2026-11-17 16:29`). Applies only to zoneless
    /// `YYYY-MM-DD HH:MM[:SS]` values; absent, those stay `undated`.
    #[serde(default)]
    pub naive_date_offset: Option<String>,
}

impl FeedSpec {
    /// The declared offset, if any and well-formed. The loader has already
    /// rejected a malformed one; a spec built by hand with a bad value is
    /// treated as declaring none.
    pub fn naive_offset(&self) -> Option<jiff::tz::Offset> {
        self.naive_date_offset.as_deref().and_then(parse_naive_offset)
    }
}

/// Strictly `±HH:MM` with a whole quarter-hour minute (`00`, `15`, `30`, `45`
/// — every real UTC offset is one) and a magnitude of at most `14:00`, the
/// widest offset in use. `-00:00` is accepted as zero. Anything else —
/// `+8:00`, `+0800`, `08:00`, surrounding spaces — is `None`, so a typo is
/// rejected at load instead of shifting every date.
pub fn parse_naive_offset(raw: &str) -> Option<jiff::tz::Offset> {
    let b = raw.as_bytes();
    if b.len() != 6 || b[3] != b':' {
        return None;
    }
    let sign = match b[0] {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let digits = [b[1], b[2], b[4], b[5]];
    if !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let d = |c: u8| i32::from(c - b'0');
    let (hh, mm) = (d(b[1]) * 10 + d(b[2]), d(b[4]) * 10 + d(b[5]));
    if mm >= 60 || mm % 15 != 0 || hh * 60 + mm > 14 * 60 {
        return None;
    }
    jiff::tz::Offset::from_seconds(sign * (hh * 3600 + mm * 60)).ok()
}

/// `~/.nullclaw/news-ai-feeds.json`. Every field is required: the crate ships
/// no feed list and no default recency window or deadline.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectConfig {
    pub feeds: Vec<FeedSpec>,
    pub max_age_hours: f64,
    pub feeds_deadline_secs: f64,
}

/// Why there is no usable feed list. The caller turns any of these into the
/// `ai_direct_feeds_empty` alert; this module never alerts.
#[derive(Debug, Clone, PartialEq)]
pub enum ConfigError {
    Missing { path: PathBuf },
    Unreadable { path: PathBuf, detail: String },
    /// The file is blank, or lists no feeds.
    Empty { path: PathBuf },
    Invalid { path: PathBuf, detail: String },
}

impl ConfigError {
    /// A stable token for traces and alert details.
    pub fn code(&self) -> &'static str {
        match self {
            ConfigError::Missing { .. } => "missing",
            ConfigError::Unreadable { .. } => "unreadable",
            ConfigError::Empty { .. } => "empty",
            ConfigError::Invalid { .. } => "invalid",
        }
    }

    pub fn path(&self) -> &Path {
        match self {
            ConfigError::Missing { path }
            | ConfigError::Unreadable { path, .. }
            | ConfigError::Empty { path }
            | ConfigError::Invalid { path, .. } => path,
        }
    }
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let path = self.path().display();
        match self {
            ConfigError::Missing { .. } => write!(f, "{path}: missing"),
            ConfigError::Empty { .. } => write!(f, "{path}: lists no feeds"),
            ConfigError::Unreadable { detail, .. } => write!(f, "{path}: unreadable: {detail}"),
            ConfigError::Invalid { detail, .. } => write!(f, "{path}: invalid: {detail}"),
        }
    }
}

/// Load and validate the feed list; `None` reads the operator's file.
pub fn load_direct_config(path: Option<&Path>) -> Result<DirectConfig, ConfigError> {
    let path = path.map(Path::to_path_buf).unwrap_or_else(ai_feeds_file);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(ConfigError::Missing { path })
        }
        Err(e) => {
            return Err(ConfigError::Unreadable {
                path,
                detail: e.to_string(),
            })
        }
    };
    if text.trim().is_empty() {
        return Err(ConfigError::Empty { path });
    }
    let cfg: DirectConfig = match serde_json::from_str(&text) {
        Ok(c) => c,
        Err(e) => {
            return Err(ConfigError::Invalid {
                path,
                detail: e.to_string(),
            })
        }
    };
    if cfg.feeds.is_empty() {
        return Err(ConfigError::Empty { path });
    }
    match validate(&cfg) {
        Ok(()) => Ok(cfg),
        Err(detail) => Err(ConfigError::Invalid { path, detail }),
    }
}

fn validate(cfg: &DirectConfig) -> Result<(), String> {
    let mut seen: HashMap<&str, usize> = HashMap::new();
    for (i, feed) in cfg.feeds.iter().enumerate() {
        let name = feed.name.as_str();
        if name.trim().is_empty() {
            return Err(format!("feeds[{i}].name is empty"));
        }
        if name.trim() != name {
            return Err(format!("feeds[{i}].name {name:?} has surrounding whitespace"));
        }
        // The name becomes the title's tail; `title_without_source` cuts at
        // the last " - ", so a name containing one would be split.
        if name.contains(" - ") {
            return Err(format!(
                "feeds[{i}].name {name:?} contains \" - \", which would break the \
                 \"<headline> - <Source>\" title invariant"
            ));
        }
        if let Some(first) = seen.insert(name, i) {
            return Err(format!("feeds[{i}].name {name:?} duplicates feeds[{first}]"));
        }
        if publisher_link(Some(&feed.url)) != Some(feed.url.as_str()) {
            return Err(format!(
                "feeds[{i}].url {:?} is not an absolute http(s) URL",
                feed.url
            ));
        }
        if feed.max_items == 0 {
            return Err(format!("feeds[{i}].max_items must be at least 1"));
        }
        if let Some(raw) = &feed.naive_date_offset {
            if parse_naive_offset(raw).is_none() {
                return Err(format!(
                    "feeds[{i}].naive_date_offset {raw:?} must be ±HH:MM, a whole quarter \
                     hour between -14:00 and +14:00"
                ));
            }
        }
    }
    let age = cfg.max_age_hours;
    if !(age.is_finite() && age > 0.0) {
        return Err(format!("max_age_hours must be a positive number, got {age:?}"));
    }
    let deadline = cfg.feeds_deadline_secs;
    if !(deadline > 0.0 && Duration::try_from_secs_f64(deadline).is_ok()) {
        return Err(format!(
            "feeds_deadline_secs must be a positive number of seconds, got {deadline:?}"
        ));
    }
    Ok(())
}

// ── §2–§3 shaping, filtering, capping ───────────────────────────────────────

/// What one feed contributed, by reason. Every fetched entry lands in exactly
/// one bucket, checked in this order: `malformed` (no usable headline or
/// Techmeme attribution) → `undated` → `future` (dated more than
/// [`FUTURE_TOLERANCE`] ahead of `now`) → `stale` → `invalid_link` → `capped`
/// (filtered fine but beyond `max_items`) → `kept`. So
/// `fetched == malformed + undated + future + stale + invalid_link + capped + kept`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct FeedCounts {
    pub fetched: usize,
    pub malformed: usize,
    pub stale: usize,
    pub undated: usize,
    pub future: usize,
    pub invalid_link: usize,
    pub capped: usize,
    pub kept: usize,
}

/// Filter-then-cap one feed's parsed entries (§3), newest first.
///
/// `now` is injected so the recency window is testable; an entry exactly
/// `max_age_hours` old is kept, and so is one exactly [`FUTURE_TOLERANCE`]
/// ahead.
pub fn select_entries(
    spec: &FeedSpec,
    entries: Vec<FeedEntry>,
    max_age_hours: f64,
    now: Timestamp,
) -> (Vec<Item>, FeedCounts) {
    let mut counts = FeedCounts {
        fetched: entries.len(),
        ..Default::default()
    };
    let max_age_ms = max_age_hours * 3_600_000.0;
    let tolerance_ms = FUTURE_TOLERANCE.as_millis() as i64;
    let naive_offset = spec.naive_offset();
    let mut survivors: Vec<(Timestamp, Item)> = Vec::new();
    for entry in entries {
        let Some((headline, source, link)) = shape(spec, &entry) else {
            counts.malformed += 1;
            continue;
        };
        let Some(published) = parse_feed_date_at(&entry.date, naive_offset) else {
            counts.undated += 1;
            continue;
        };
        let age_ms = now.as_millisecond() - published.as_millisecond();
        if -age_ms > tolerance_ms {
            counts.future += 1;
            continue;
        }
        if age_ms as f64 > max_age_ms {
            counts.stale += 1;
            continue;
        }
        let Some(link) = publisher_link(Some(&link)) else {
            counts.invalid_link += 1;
            continue;
        };
        survivors.push((
            // Within tolerance a future date sorts as `now`, never ahead of it.
            published.min(now),
            Item {
                title: format!("{headline} - {source}"),
                link: link.to_string(),
                pub_date: entry.date,
                decoded_url: None,
                source,
            },
        ));
    }
    // Stable: equal timestamps keep the feed's own order.
    survivors.sort_by_key(|(published, _)| std::cmp::Reverse(*published));
    counts.capped = survivors.len().saturating_sub(spec.max_items);
    survivors.truncate(spec.max_items);
    counts.kept = survivors.len();
    (survivors.into_iter().map(|(_, item)| item).collect(), counts)
}

/// `(headline, source, link)` for one entry, or `None` when it is malformed.
fn shape(spec: &FeedSpec, entry: &FeedEntry) -> Option<(String, String, String)> {
    let title = collapse_whitespace(&entry.title);
    match spec.kind {
        FeedKind::Plain => {
            (!title.is_empty()).then(|| (title, spec.name.clone(), entry.link.clone()))
        }
        FeedKind::Techmeme => {
            let (headline, publication) = techmeme_attribution(&title)?;
            // No article link is not a malformed headline: it is an item
            // with nothing to point at, counted as `invalid_link`.
            let link = techmeme_link(&entry.description).unwrap_or_default();
            Some((headline, publication, link))
        }
    }
}

fn collapse_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub use crate::feed::parse_feed_date;

/// Split Techmeme's `"Headline (Author/Publication)"` into headline and
/// publication. The attribution is the trailing balanced parenthesis, so a
/// headline may itself contain one — `Apple (AAPL) beats … (Jane/WSJ)`. With
/// no author it is just `(Publication)`.
pub fn techmeme_attribution(title: &str) -> Option<(String, String)> {
    let body = title.trim().strip_suffix(')')?;
    let mut depth = 0usize;
    let mut open = None;
    for (i, c) in body.char_indices().rev() {
        match c {
            ')' => depth += 1,
            '(' if depth == 0 => {
                open = Some(i);
                break;
            }
            '(' => depth -= 1,
            _ => {}
        }
    }
    let open = open?;
    let headline = body[..open].trim();
    let publication = body[open + 1..].rsplit('/').next().unwrap_or("").trim();
    if headline.is_empty() || publication.is_empty() || publication.contains(" - ") {
        return None;
    }
    Some((headline.to_string(), publication.to_string()))
}

static HREF: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)\bhref\s*=\s*(?:"([^"]*)"|'([^']*)')"#).expect("href pattern")
});

/// The article a Techmeme item is about: the first `HREF` in its description
/// whose host is not Techmeme's and whose path is not the site root.
///
/// The root-path exclusion is not in the spec's wording. Every item recorded
/// on 2026-11-17 opens with an image linked to the article, but an item
/// without an image would open with the outlet's homepage link
/// (`<A HREF="https://www.ft.com/">Financial Times</A>`), and a homepage is
/// never the story.
pub fn techmeme_link(description: &str) -> Option<String> {
    HREF.captures_iter(description).find_map(|cap| {
        let raw = cap.get(1).or_else(|| cap.get(2))?.as_str();
        let url = raw.trim().replace("&amp;", "&");
        let (netloc, path, _) = split_url(&url)?;
        let host = netloc
            .rsplit('@')
            .next()
            .unwrap_or("")
            .split(':')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        let techmeme = host == "techmeme.com" || host.ends_with(".techmeme.com");
        let root = path.is_empty() || path == "/";
        (!host.is_empty() && !techmeme && !root).then_some(url)
    })
}

// ── §4 fetching, §7 the report ───────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedOutcome {
    Ok,
    HttpError,
    Timeout,
    Oversize,
    ParseError,
}

/// One feed's line in the `ai_direct_feeds` trace. `detail` says which HTTP
/// status, parse failure or limit was hit; it never carries a URL.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FeedReport {
    pub name: String,
    pub outcome: FeedOutcome,
    pub detail: Option<String>,
    #[serde(flatten)]
    pub counts: FeedCounts,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DirectReport {
    /// In config order.
    pub feeds: Vec<FeedReport>,
    pub total_kept: usize,
    pub elapsed_ms: u64,
}

impl DirectReport {
    /// The `ai_direct_feeds` event's fields, exactly as traced.
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

fn failed(name: &str, outcome: FeedOutcome, detail: Option<String>) -> FeedReport {
    FeedReport {
        name: name.to_string(),
        outcome,
        detail,
        counts: FeedCounts::default(),
    }
}

/// Parse and filter one feed's response body (§3). Public so a recorded body
/// can be pushed through exactly what a fetched one goes through.
pub fn process_body(
    spec: &FeedSpec,
    body: &[u8],
    max_age_hours: f64,
    now: Timestamp,
) -> (FeedReport, Vec<Item>) {
    if body.len() > MAX_BODY_BYTES {
        let detail = format!("body exceeds {MAX_BODY_BYTES} bytes");
        return (failed(&spec.name, FeedOutcome::Oversize, Some(detail)), Vec::new());
    }
    let text = String::from_utf8_lossy(body);
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    match parse_feed_all_at(text, spec.naive_offset()) {
        Err(e) => (failed(&spec.name, FeedOutcome::ParseError, Some(e)), Vec::new()),
        Ok(entries) => {
            let (items, counts) = select_entries(spec, entries, max_age_hours, now);
            let report = FeedReport {
                name: spec.name.clone(),
                outcome: FeedOutcome::Ok,
                detail: None,
                counts,
            };
            (report, items)
        }
    }
}

fn is_timeout_io(kind: std::io::ErrorKind) -> bool {
    matches!(kind, std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock)
}

fn transport_timed_out(t: &ureq::Transport) -> bool {
    let mut source = std::error::Error::source(t);
    while let Some(e) = source {
        if let Some(io) = e.downcast_ref::<std::io::Error>() {
            if is_timeout_io(io.kind()) {
                return true;
            }
        }
        source = e.source();
    }
    false
}

/// GET one feed; at most `MAX_BODY_BYTES + 1` bytes are read, so an oversize
/// body is detected without being held whole. gzip is decoded by `ureq`.
fn fetch_body(url: &str, timeout: Duration) -> Result<Vec<u8>, (FeedOutcome, Option<String>)> {
    let response = match claw_core::http::agent(timeout)
        .get(url)
        .set("User-Agent", USER_AGENT)
        .call()
    {
        Ok(r) => r,
        Err(ureq::Error::Transport(t)) if transport_timed_out(&t) => {
            return Err((FeedOutcome::Timeout, None))
        }
        Err(e) => {
            return Err((
                FeedOutcome::HttpError,
                Some(claw_core::http::error_class(&e)),
            ))
        }
    };
    // A 2xx other than 200 (a 204, say) arrives as `Ok` but carries no feed.
    if response.status() != 200 {
        return Err((
            FeedOutcome::HttpError,
            Some(format!("HTTP {}", response.status())),
        ));
    }
    let mut body = Vec::new();
    match response
        .into_reader()
        .take(MAX_BODY_BYTES as u64 + 1)
        .read_to_end(&mut body)
    {
        Ok(_) => Ok(body),
        Err(e) if is_timeout_io(e.kind()) => Err((FeedOutcome::Timeout, None)),
        Err(e) => Err((
            FeedOutcome::HttpError,
            Some(format!("body read failed: {:?}", e.kind())),
        )),
    }
}

/// Fetch every configured feed concurrently, one thread each, and return the
/// kept items — feed by feed in config order, each feed newest first — with
/// the per-feed report. Also writes that report as the `ai_direct_feeds`
/// trace event.
///
/// The whole fetch stops at `feeds_deadline_secs`; a feed still pending then
/// is reported `timeout` and its thread is abandoned (each request is itself
/// bounded by [`FEED_TIMEOUT`], so an abandoned worker cannot live long). An
/// empty result is not an error here: the report says why, and the caller
/// decides what an empty AI section means.
pub fn fetch_direct_ai(cfg: &DirectConfig, now: Timestamp) -> (Vec<Item>, DirectReport) {
    let started = Instant::now();
    let budget = Duration::try_from_secs_f64(cfg.feeds_deadline_secs).unwrap_or(Duration::ZERO);
    let deadline = started + budget;

    let (tx, rx) = mpsc::channel::<(usize, (FeedReport, Vec<Item>))>();
    let mut slots: Vec<Option<(FeedReport, Vec<Item>)>> = cfg.feeds.iter().map(|_| None).collect();
    let mut pending = 0usize;
    for (idx, spec) in cfg.feeds.iter().enumerate() {
        let tx = tx.clone();
        let worker_spec = spec.clone();
        let max_age_hours = cfg.max_age_hours;
        let spawned = std::thread::Builder::new()
            .name(format!("direct-feed-{idx}"))
            .spawn(move || {
                let spec = worker_spec;
                let result = match fetch_body(&spec.url, FEED_TIMEOUT) {
                    Ok(body) => process_body(&spec, &body, max_age_hours, now),
                    Err((outcome, detail)) => (failed(&spec.name, outcome, detail), Vec::new()),
                };
                let _ = tx.send((idx, result));
            });
        match spawned {
            Ok(_) => pending += 1,
            Err(e) => {
                let detail = Some(format!("thread spawn failed: {e}"));
                slots[idx] = Some((failed(&spec.name, FeedOutcome::HttpError, detail), Vec::new()));
            }
        }
    }
    drop(tx);

    while pending > 0 {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        match rx.recv_timeout(remaining) {
            Ok((idx, result)) => {
                slots[idx] = Some(result);
                pending -= 1;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => break,
            // Every worker has finished or died; a dead one left no result.
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let deadline_hit = Instant::now() >= deadline;

    let mut items = Vec::new();
    let mut feeds = Vec::with_capacity(cfg.feeds.len());
    for (spec, slot) in cfg.feeds.iter().zip(slots) {
        let (report, feed_items) = slot.unwrap_or_else(|| {
            let detail = (!deadline_hit).then(|| "worker exited without a result".to_string());
            (failed(&spec.name, FeedOutcome::Timeout, detail), Vec::new())
        });
        items.extend(feed_items);
        feeds.push(report);
    }
    let report = DirectReport {
        total_kept: feeds.iter().map(|f| f.counts.kept).sum(),
        feeds,
        elapsed_ms: started.elapsed().as_millis() as u64,
    };
    log_trace("ai_direct_feeds", report.to_json());
    (items, report)
}
