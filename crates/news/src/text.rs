//! Headline text handling: source stripping, tokenising, dedup, clustering.
//!
//! All pure. This is the deterministic half of the skill — everything an LLM
//! is later asked to judge is first narrowed by these rules.

use crate::validate::count_cjk;
use std::collections::HashSet;

/// Tokens too common to distinguish one headline from another.
const STOPWORDS: &[&str] = &[
    "the", "a", "an", "and", "or", "to", "of", "for", "in", "on", "with", "new", "ai", "is", "are",
    "be", "at", "from", "your", "you", "our", "its", "it", "more", "all", "how", "why", "what",
    "as", "by", "this", "的", "是", "了", "在", "和", "與", "及", "也", "都", "就", "而", "對",
    "為", "以", "從", "把", "被", "將", "這", "那", "有", "沒",
];

/// CJK bigrams that appear in almost every business headline.
const CJK_STOP_BIGRAMS: &[&str] = &[
    "公司", "發布", "布新", "新產", "產品", "股價", "上漲", "下跌",
];

/// How many shared tokens make two headlines the same event.
pub const CLUSTER_OVERLAP: usize = 2;

fn is_cjk(c: char) -> bool {
    matches!(c, '\u{3400}'..='\u{9fff}' | '\u{f900}'..='\u{faff}')
}

/// The single-character CJK stopwords, as a set of chars.
fn cjk_stop_chars() -> HashSet<char> {
    STOPWORDS
        .iter()
        .filter(|w| w.chars().count() == 1)
        .filter_map(|w| w.chars().next())
        .filter(|c| is_cjk(*c))
        .collect()
}

/// Google News appends " - Source" to every title.
pub fn extract_source_name(title: &str) -> String {
    match title.rfind(" - ") {
        Some(i) => title[i + 3..].trim().to_string(),
        None => String::new(),
    }
}

pub fn title_without_source(title: &str) -> &str {
    match title.rfind(" - ") {
        Some(i) => &title[..i],
        None => title,
    }
}

/// Standing lead-ins a headline carries but the story does not: `獨家：`,
/// `Exclusive |`, `【社論】`, `華爾街日報》`.
///
/// Google News splits a query on whitespace and on `：`/`|` and **ANDs** the
/// fragments, so a lead-in stops being a label and becomes a *required term*
/// that only the outlet which ran the exclusive ever uses. Measured 2026-09-16
/// against Google News RSS, zh-TW edition unless noted:
///
/// | query | items |
/// |---|---|
/// | `獨家：台積電2奈米提前量產` | 0 |
/// | `台積電2奈米提前量產` | 7 |
/// | `【獨家】台積電2奈米提前量產` | 0 |
/// | `獨家：美國施壓墨西哥阻擋中國` | 0 |
/// | `美國施壓墨西哥阻擋中國` | 5 |
/// | `Exclusive | U.S. Pressures Mexico to Box Out China's AI Hardware Exports` (en-US) | 1 |
/// | `U.S. Pressures Mexico to Box Out China's AI Hardware Exports` (en-US) | 3 |
const LEAD_IN_LABELS: &[&str] = &[
    // Chinese news furniture.
    "獨家",
    "獨家報導",
    "獨家專訪",
    "快訊",
    "快報",
    "即時",
    "即時新聞",
    "最新",
    "不斷更新",
    "更新",
    "重磅",
    "焦點",
    "影",
    "影片",
    "影音",
    "影音報導",
    "圖輯",
    "照片",
    "專訪",
    "現場",
    "直擊",
    "深度",
    "分析",
    "評論",
    "社論",
    "風評",
    "專欄",
    "特別報導",
    "懶人包",
    "整理包",
    "話題",
    "熱門",
    "早安世界",
    "快報頭條",
    // English news furniture.
    "exclusive",
    "breaking",
    "video",
    "watch",
    "opinion",
    "update",
    "live",
    "updates",
    "analysis",
    "interview",
    "review",
    "explainer",
    "editorial",
    "column",
    "photos",
    "recap",
];

/// Characters a bare lead-in label is followed by.
const LEAD_IN_SEPARATORS: [char; 5] = ['：', ':', '｜', '|', '／'];

/// Deliberately NOT here: `陸股`, `台股`, `美股`, `港股`, `盤中`, `收盤`. They
/// read like furniture but they are also *terms the coverage uses*, so
/// stripping them loses results rather than finding them — measured 2026-09-16,
/// `台股：外資買超 台積電領漲` scored 37 items whole and 10 stripped, `盤中：…`
/// 31 and 5. A label only belongs in this list when it reliably empties the
/// query, which is what `獨家`/`快訊`/`Exclusive`/`Video` do.
///
/// How far into the headline a lead-in may start. Bounds the scan so a colon
/// inside a long headline is never mistaken for a label separator.
const LEAD_IN_MAX_CHARS: usize = 16;

fn is_lead_in_label(head: &str) -> bool {
    let head = head.trim();
    !head.is_empty()
        && head.chars().count() <= LEAD_IN_MAX_CHARS
        && LEAD_IN_LABELS.iter().any(|l| head.eq_ignore_ascii_case(l))
}

/// Drop one leading label, or `None` when the headline opens with the story.
///
/// Three shapes, all seen in the live feeds:
/// - bracketed label: `【社論】…`, `[Video] …`
/// - bare label plus separator: `獨家：…`, `Exclusive | …`, `影／…`
/// - dangling close, the opener having been dropped upstream: `華爾街日報》…`,
///   `早安世界》…`, `MLB》…`
///
/// The first two are gated on the label being a *known* label, and the third on
/// there being no opener in the prefix. Both gates exist for the same reason: a
/// bracket is also how a headline names the work or product it is about, and
/// `《Apex英雄》9/22聯動《快打旋風6》` stripped to `9/22聯動《快打旋風6》` — or
/// `不只《蘭香如故》好看！` to `好看！` — loses the one token the search needs.
fn strip_one_lead_in(s: &str) -> Option<&str> {
    for (open, close) in [('【', '】'), ('[', ']')] {
        if s.starts_with(open) {
            if let Some((i, _)) = s
                .char_indices()
                .take(LEAD_IN_MAX_CHARS)
                .find(|(_, c)| *c == close)
            {
                if !is_lead_in_label(&s[open.len_utf8()..i]) {
                    break;
                }
                let rest = &s[i + close.len_utf8()..];
                if !rest.trim().is_empty() {
                    return Some(rest);
                }
            }
            break;
        }
    }

    if let Some((i, sep)) = s
        .char_indices()
        .take(LEAD_IN_MAX_CHARS)
        .find(|(_, c)| LEAD_IN_SEPARATORS.contains(c))
    {
        if is_lead_in_label(&s[..i]) {
            let rest = &s[i + sep.len_utf8()..];
            if !rest.trim().is_empty() {
                return Some(rest);
            }
        }
    }

    let close_at = s
        .char_indices()
        .take(LEAD_IN_MAX_CHARS)
        .find(|(_, c)| matches!(c, '》' | '】' | ']'));
    if let Some((i, close)) = close_at {
        let prefix = &s[..i];
        // An opener in the prefix means this close belongs to it — the headline
        // is about the bracketed work, not labelled by the text before it.
        if !prefix.contains(['《', '【', '[']) {
            let rest = &s[i + close.len_utf8()..];
            if !rest.trim().is_empty() {
                return Some(rest);
            }
        }
    }

    None
}

/// The text to search for when hunting another outlet's coverage of a story.
///
/// The source suffix goes, as everywhere else; then any standing lead-in goes,
/// repeatedly, because headlines stack them (`Video: Opinion | How China Sees
/// the A.I. Race`). What is left is the part of the headline another outlet
/// would also have used — which is the only part a full-text search can match.
///
/// Used for the paywall replacement lookup. Never for display: the digest shows
/// the headline the model wrote, and this is only a query string.
pub fn replacement_query(title: &str) -> String {
    let mut s = title_without_source(title).trim();
    while let Some(rest) = strip_one_lead_in(s) {
        let rest = rest.trim();
        if rest.is_empty() {
            break;
        }
        s = rest;
    }
    s.trim().to_string()
}

/// Whether a headline is written in Chinese, judged after the source suffix is
/// removed.
///
/// One definition, because two callers key off it: the story gate decides
/// `Undecidable` from it, and the search layer picks its edition and market from
/// it. A suffix is not part of the headline — Google appends `" - 自由時報"` to
/// an English headline carried by a Taiwanese outlet, and counting that would
/// call the headline Chinese and send the search to the wrong edition.
pub fn is_cjk_headline(title: &str) -> bool {
    count_cjk(title_without_source(title)) >= 2
}

/// Latin tokens of a headline or body: lowercase `[a-z0-9]+` runs of two or
/// more characters containing at least one letter. Pure digit runs ("1.2")
/// carry no entity identity and are dropped. Unlike `topic_words` this keeps
/// short tokens and has no stopword list — the enrichment validator needs
/// every Latin token accounted for, not just the distinctive ones.
pub fn latin_tokens(s: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    let mut cur = String::new();
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() {
            cur.push(ch.to_ascii_lowercase());
        } else if latin_token_ok(&cur) {
            out.insert(std::mem::take(&mut cur));
        } else {
            cur.clear();
        }
    }
    if latin_token_ok(&cur) {
        out.insert(cur);
    }
    out
}

fn latin_token_ok(t: &str) -> bool {
    t.len() >= 2 && t.chars().any(|c| c.is_ascii_alphabetic())
}

/// Significant headline tokens, used for deterministic event clustering.
///
/// Latin runs of three or more characters, plus every CJK *bigram* — Chinese
/// headlines have no spaces, so a bigram is the smallest unit that carries
/// meaning. Bigrams touching a stop character, and a short list of
/// business-headline filler, are dropped.
pub fn topic_words(title: &str) -> HashSet<String> {
    let text = title_without_source(title).to_lowercase();
    let stop: HashSet<&str> = STOPWORDS.iter().copied().collect();
    let cjk_stop = cjk_stop_chars();
    let bigram_stop: HashSet<&str> = CJK_STOP_BIGRAMS.iter().copied().collect();

    let mut words = HashSet::new();

    // Latin/digit runs, matching the Python's `[a-z0-9.]+`.
    let mut cur = String::new();
    for ch in text.chars() {
        if ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '.' {
            cur.push(ch);
        } else {
            if cur.chars().count() > 2 && !stop.contains(cur.as_str()) {
                words.insert(std::mem::take(&mut cur));
            } else {
                cur.clear();
            }
        }
    }
    if cur.chars().count() > 2 && !stop.contains(cur.as_str()) {
        words.insert(cur);
    }

    // CJK bigrams, taken within each unbroken run.
    let mut run: Vec<char> = Vec::new();
    let flush = |run: &mut Vec<char>, words: &mut HashSet<String>| {
        for pair in run.windows(2) {
            if cjk_stop.contains(&pair[0]) || cjk_stop.contains(&pair[1]) {
                continue;
            }
            let s: String = pair.iter().collect();
            if bigram_stop.contains(s.as_str()) {
                continue;
            }
            words.insert(s);
        }
        run.clear();
    };
    for ch in text.chars() {
        if is_cjk(ch) {
            run.push(ch);
        } else {
            flush(&mut run, &mut words);
        }
    }
    flush(&mut run, &mut words);

    words
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Item {
    pub title: String,
    pub link: String,
    pub pub_date: String,
    /// The real article URL, once the precheck has resolved a Google News
    /// redirect or unwrapped a Bing click tracker. `None` means the link is
    /// still whatever the feed gave us.
    pub decoded_url: Option<String>,
    pub source: String,
}

impl Item {
    /// The article URL to reason about: the decoded one when we have it.
    pub fn effective_url(&self) -> &str {
        self.decoded_url.as_deref().unwrap_or(&self.link)
    }
}

/// Drop repeated titles, case-insensitively, keeping first occurrence.
pub fn dedup(items: &[Item]) -> Vec<Item> {
    let mut seen: HashSet<String> = HashSet::new();
    items
        .iter()
        .filter(|it| seen.insert(it.title.to_lowercase()))
        .cloned()
        .collect()
}

/// Group headlines covering the same event, by token overlap with the seed.
///
/// Compared against the *seed* only, not the whole group — a group that
/// compared against its union would drift, each new member widening what counts
/// as the same event until unrelated headlines join.
///
/// A seed with fewer than CLUSTER_OVERLAP tokens can never grow. That is
/// intentional: a one-token headline is too weak to anchor an event.
///
/// Largest group first.
pub fn cluster(items: &[Item]) -> Vec<Vec<Item>> {
    struct Group {
        seed: HashSet<String>,
        items: Vec<Item>,
    }
    let mut groups: Vec<Group> = Vec::new();

    for item in items {
        let words = topic_words(&item.title);
        let mut placed = false;
        for g in groups.iter_mut() {
            if words.intersection(&g.seed).count() >= CLUSTER_OVERLAP {
                g.items.push(item.clone());
                placed = true;
                break;
            }
        }
        if !placed {
            groups.push(Group {
                seed: words,
                items: vec![item.clone()],
            });
        }
    }

    // Stable sort, so equal-sized groups keep the order they were found in —
    // an unstable sort here would make the digest reorder between runs on
    // identical input.
    groups.sort_by_key(|g| std::cmp::Reverse(g.items.len()));
    groups.into_iter().map(|g| g.items).collect()
}

/// Take the first `per_cluster` from each already-ranked group.
pub fn pick_representatives(clusters: &[Vec<Item>], per_cluster: usize) -> Vec<Item> {
    clusters
        .iter()
        .flat_map(|g| g.iter().take(per_cluster).cloned())
        .collect()
}
