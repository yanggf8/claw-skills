//! Checking what the model sent back.
//!
//! An LLM asked for "one bullet per item, each prefixed `#N`, in Chinese" will
//! sometimes answer with reasoning prose, sometimes drop the markers, and
//! sometimes answer in English. These gates decide whether a reply is usable
//! before it becomes a delivered digest.

use std::collections::HashSet;

/// The "nothing relevant today" sentinel, and its body without the bullet dash.
///
/// One definition, because three gates key off it: [`news_bullet_lines`] and
/// [`content_lines`] filter it out, and [`is_no_news_answer`] recognises it as
/// a deliberate answer rather than a malformed one.
pub const NO_NEWS_BODY: &str = "今日無相關新聞";
pub const NO_NEWS: &str = "- 今日無相關新聞";

fn is_cjk_common(c: char) -> bool {
    ('\u{4e00}'..='\u{9fff}').contains(&c)
}

pub fn count_cjk(text: &str) -> usize {
    text.chars().filter(|c| is_cjk_common(*c)).count()
}

/// Parse a leading `#N` marker.
///
/// `#12` matches; `#12,` does not. The negative lookahead on a comma exists
/// because a model listing sources inline ("見 #3, #7") would otherwise have
/// its prose counted as a marked bullet.
pub fn leading_marker(line: &str) -> Option<u32> {
    leading_marker_span(line).map(|(n, _)| n)
}

/// The marker id and the byte offset in `line` just past its digits.
///
/// Hand-rolled rather than a regex because `\b(?!,)` needs a lookahead, which
/// the `regex` crate does not have. The renderer needs the offset too, so both
/// callers share one parser instead of drifting apart.
pub fn leading_marker_span(line: &str) -> Option<(u32, usize)> {
    let lead = line.len() - line.trim_start().len();
    let s = line.trim_start();
    let dash = match s.strip_prefix('-') {
        Some(rest) => 1 + (rest.len() - rest.trim_start().len()),
        None => 0,
    };
    let s = &s[dash..];
    let rest = s.strip_prefix('#')?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return None;
    }
    let end = lead + dash + 1 + digits.len();
    // `\b(?!,)` — the character after the digits must not be a comma, and must
    // not be another word character.
    match rest[digits.len()..].chars().next() {
        Some(',') => None,
        Some(c) if c.is_alphanumeric() || c == '_' => None,
        // Saturating, not `.ok()`. Whether a marker is *syntactically* present
        // is a separate question from what number it names, and conflating them
        // costs more than the overflow does: an id too large for u32 would
        // otherwise leave the marker text sitting in the bullet body, where the
        // language gate reads it as a long non-Chinese prefix and rejects the
        // whole digest. Python has no such ceiling. An id this large is never in
        // `numbered` either way, so saturating keeps both answers right.
        _ => Some((digits.parse().unwrap_or(u32::MAX), end)),
    }
}

/// Strip a leading `#N` marker and surrounding whitespace.
pub fn strip_marker_prefix(line: &str) -> String {
    match leading_marker(line) {
        None => line.trim().to_string(),
        Some(_) => {
            let s = line.trim_start();
            let s = s.strip_prefix('-').map(str::trim_start).unwrap_or(s);
            let rest = &s[1..]; // past '#'
            let after: String = rest
                .chars()
                .skip_while(char::is_ascii_digit)
                .collect();
            after.trim().to_string()
        }
    }
}

const RULES: [&str; 3] = ["---", "--", "***"];

/// News bullets that should carry a source marker.
///
/// Deliberately narrower than [`content_lines`]. Around nine call sites use
/// this as an emptiness check — "no selectable news items" routes to the
/// 今日無相關新聞 placeholder — and for language and dedup statistics.
/// Widening it would flip those meanings: a reply that is only chain-of-thought
/// currently yields nothing here and correctly reaches the placeholder path.
pub fn news_bullet_lines(summary: &str) -> Vec<&str> {
    summary
        .lines()
        .filter(|line| {
            let s = line.trim();
            if RULES.contains(&s) {
                return false;
            }
            if !s.starts_with('-') && leading_marker(s).is_none() {
                return false;
            }
            let body = s.strip_prefix('-').map(str::trim).unwrap_or(s);
            !body.is_empty() && !body.starts_with("...") && !body.contains(NO_NEWS_BODY)
        })
        .collect()
}

/// A line that is nothing but one `[bracket]` — instrumentation, stripped later
/// during link attachment, so it must not fail the shape gate.
///
/// The interior may contain a `[` but not a `]`: `[a[b]` is one bracket,
/// `[a]b]` is a bracket followed by text and stays visible to the gate.
fn is_bracket_noise(s: &str) -> bool {
    let mut chars = s.chars();
    if chars.next() != Some('[') || !s.ends_with(']') || s.chars().count() < 2 {
        return false;
    }
    let inner_len = s.chars().count() - 2;
    !chars.take(inner_len).any(|c| c == ']')
}

/// Everything the model emitted that is not framing.
///
/// Separate from [`news_bullet_lines`], and wider on purpose: this exists to
/// make un-bulleted reasoning prose VISIBLE to the shape gate, so a reply that
/// explains itself instead of answering is rejected rather than delivered.
///
/// Pure `[bracket]` instrumentation is excluded — it is stripped later during
/// link attachment and must not fail the gate.
pub fn content_lines(summary: &str) -> Vec<&str> {
    summary
        .lines()
        .filter(|line| {
            let s = line.trim();
            if s.is_empty() || RULES.contains(&s) {
                return false;
            }
            if s.starts_with("**") && s.ends_with("**") {
                return false;
            }
            if is_bracket_noise(s) {
                return false;
            }
            let body = s.strip_prefix('-').map(str::trim).unwrap_or(s);
            !body.contains(NO_NEWS_BODY) && !body.starts_with("...")
        })
        .collect()
}

/// Decoration a model wraps the sentinel in — quotes, bullets, a full stop.
///
/// The prompt names the sentinel as 「- 今日無相關新聞」, quoting the bullet dash
/// along with the body, so a model copying the instruction verbatim emits both
/// the quotes and the dash. Stripping is iterative for that reason: one pass
/// removes the quotes, the next the dash it was hiding.
const SENTINEL_DECORATION: &str = "「」『』｢｣＂＇\"'。．.*＊-—–";

/// A line reduced to its text, with any bullet and quoting peeled away.
fn undecorated(line: &str) -> &str {
    let mut s = line.trim();
    loop {
        let peeled = s.trim_matches(|c: char| SENTINEL_DECORATION.contains(c)).trim();
        if peeled == s {
            return s;
        }
        s = peeled;
    }
}

/// True when the model took the prompt's explicit "nothing relevant today"
/// option instead of failing to follow the format.
///
/// Only the custom-topic prompt offers that option, and it names exactly
/// [`NO_NEWS`]. Without this gate the sentinel reaches [`marker_stats`], which
/// counts zero news bullets — [`news_bullet_lines`] filters the sentinel out —
/// reports `total == 0`, and the caller treats a correct answer as a protocol
/// violation: it falls back to the raw listing and alerts the operator, which
/// is the opposite of what the model said.
///
/// Both halves are load-bearing:
///
/// * [`content_lines`] must find nothing, so a reply that answers *and then
///   argues* still falls back — it has not answered the question. This reuses
///   the pipeline's own notion of framing, so markdown rules and bold headings
///   around the sentinel do not count against it.
/// * Some line must reduce to exactly the sentinel. `content_lines` drops every
///   line merely *containing* it, so without this half
///   「今日無相關新聞，但有一則值得一提」 would sail through as "no content".
pub fn is_no_news_answer(summary: &str) -> bool {
    content_lines(summary).is_empty()
        && summary.lines().any(|l| undecorated(l) == NO_NEWS_BODY)
}

/// Every content line must carry a marker naming an item we offered.
///
/// An empty reply fails: a model that returned nothing has not passed a shape
/// check, it has skipped one.
pub fn shape_ok(summary: &str, numbered: &HashSet<u32>) -> bool {
    let content = content_lines(summary);
    if content.is_empty() {
        return false;
    }
    content
        .iter()
        .all(|l| leading_marker(l).is_some_and(|n| numbered.contains(&n)))
}

/// (bullets carrying a valid marker, total news bullets).
pub fn marker_stats(summary: &str, numbered: &HashSet<u32>) -> (usize, usize) {
    let bullets = news_bullet_lines(summary);
    let marked = bullets
        .iter()
        .filter(|l| leading_marker(l).is_some_and(|n| numbered.contains(&n)))
        .count();
    (marked, bullets.len())
}

/// Marker ids in the order the model used them, without repeats.
pub fn leading_marker_ids(summary: &str, numbered: &HashSet<u32>) -> Vec<u32> {
    let mut seen = HashSet::new();
    news_bullet_lines(summary)
        .iter()
        .filter_map(|l| leading_marker(l))
        .filter(|n| numbered.contains(n) && seen.insert(*n))
        .collect()
}

/// English adverbs that are never proper nouns.
///
/// Their presence means the model wrote English prose and then part-translated
/// it, which reads worse than either language alone.
const FORBIDDEN_ENGLISH: &[&str] = &[
    "increasingly", "significantly", "rapidly", "notably", "effectively", "essentially",
    "generally", "particularly", "specifically", "primarily", "ultimately", "eventually",
    "additionally", "furthermore", "moreover", "however", "therefore", "consequently",
    "meanwhile", "subsequently", "previously", "currently", "recently", "approximately",
    "potentially",
];

/// (bullets that look Chinese, total news bullets).
///
/// "Looks Chinese" is two or more CJK characters with the first appearing
/// within the opening 18 — a bullet that starts in English and turns Chinese
/// halfway does not count, which is the failure this measures.
pub fn language_stats(summary: &str) -> (usize, usize) {
    let bullets = news_bullet_lines(summary);
    let chinese = bullets
        .iter()
        .filter(|l| {
            let body = strip_marker_prefix(l);
            let first = body.chars().position(is_cjk_common);
            count_cjk(&body) >= 2 && matches!(first, Some(i) if i <= 18)
        })
        .count();
    (chinese, bullets.len())
}

/// Four fifths of bullets must read as Chinese, and none may carry an English
/// adverb from the list above.
pub fn language_ok(summary: &str) -> bool {
    let (chinese, total) = language_stats(summary);
    if total == 0 {
        return false;
    }
    // chinese/total >= 4/5, in integers to avoid a float comparison deciding a
    // gate.
    if chinese * 5 < total * 4 {
        return false;
    }
    let forbidden: HashSet<&str> = FORBIDDEN_ENGLISH.iter().copied().collect();
    !news_bullet_lines(summary).iter().any(|l| {
        strip_marker_prefix(l).split_whitespace().any(|tok| {
            let cleaned = tok.trim_matches(|c: char| c.is_ascii_punctuation());
            forbidden.contains(cleaned.to_lowercase().as_str())
        })
    })
}

/// Company names from [`crate::config::PROTECTED_NAMES`] that a source
/// headline carried and its delivered line does not.
///
/// Two different faults land here and the caller should not assume which.
/// Mistranslation is the one it was built for — 「擁抱臉書」 for Hugging Face.
/// The first real run instead caught the other: "Meta, Anthropic, Google,
/// OpenAI to meet Trump officials" shipped as 「美國多家科技巨頭將與川普政府
/// 會談」, every name collapsed into a generic phrase. Nothing was translated,
/// and the reader still lost the fact. Hence "did not reach the reader"
/// rather than "was translated".
///
/// Returns `(name, link)` per loss. The join is the article link, not the
/// `#N` marker: `attach_numbered_links` strips the marker and writes the
/// item's URL into the rendered line, so by the time a digest exists the URL
/// is the only thing tying a line back to the headline it came from.
///
/// This never gates delivery, and the caller must not make it one. A rejected
/// section falls back to an untranslated English dump and raises
/// `section_fallback_used`, which is a worse digest and a false alarm — both
/// strictly worse than one company name rendered in Chinese. It exists so a
/// recurrence is countable instead of depending on a reader noticing.
///
/// A headline that was never selected produces no line and is not a loss;
/// only items whose link actually appears in the digest are checked.
pub fn dropped_protected_names(
    sources: &[(String, String)],
    digest_lines: &[String],
) -> Vec<(String, String)> {
    let mut lost = Vec::new();
    for (title, link) in sources {
        if link.is_empty() {
            continue;
        }
        let names: Vec<&&str> = crate::config::PROTECTED_NAMES
            .iter()
            .filter(|n| title.contains(**n))
            .collect();
        if names.is_empty() {
            continue;
        }
        // A paywall replacement renders two lines for one item, both carrying
        // the link; surviving in either is enough.
        let rendered: Vec<&String> = digest_lines.iter().filter(|l| l.contains(link)).collect();
        if rendered.is_empty() {
            continue;
        }
        for name in names {
            if !rendered.iter().any(|l| l.contains(*name)) {
                lost.push(((*name).to_string(), link.clone()));
            }
        }
    }
    lost
}

/// Telegram's legacy Markdown parser rejects a message with an unmatched `*`
/// or `_`. Taiwanese stock notation uses `*` literally ("長科*成關鍵受惠股"),
/// so substitute the full-width forms: visually identical, no control meaning.
///
/// Headline body text only — never scaffolding, link markup or chunk prefixes.
pub fn neutralize_markdown(text: &str) -> String {
    text.replace('*', "＊").replace('_', "＿")
}
