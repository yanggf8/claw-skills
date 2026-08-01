//! Headline text handling: source stripping, tokenising, dedup, clustering.
//!
//! All pure. This is the deterministic half of the skill — everything an LLM
//! is later asked to judge is first narrowed by these rules.

use std::collections::HashSet;

/// Tokens too common to distinguish one headline from another.
const STOPWORDS: &[&str] = &[
    "the", "a", "an", "and", "or", "to", "of", "for", "in", "on", "with", "new", "ai", "is",
    "are", "be", "at", "from", "your", "you", "our", "its", "it", "more", "all", "how", "why",
    "what", "as", "by", "this", "的", "是", "了", "在", "和", "與", "及", "也", "都", "就", "而",
    "對", "為", "以", "從", "把", "被", "將", "這", "那", "有", "沒",
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
