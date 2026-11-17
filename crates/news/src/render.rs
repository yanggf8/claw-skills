//! Turning validated model output into the message that gets sent.
//!
//! Link attachment, length trimming, chunking, and the Markdown safety probe.
//!
//! Every length here is counted in **characters**, never bytes. Python's `len`
//! counts characters and the limits were chosen against that; measuring a
//! Chinese digest in bytes would cut it to roughly a third of its intended
//! size.

use crate::config::TELEGRAM_RAW_CHUNK_LIMIT;
use crate::text::{title_without_source, Item};
use crate::trace::log_trace;
use crate::validate::{count_cjk, leading_marker_span, neutralize_markdown};
use regex::Regex;
use serde_json::json;
use std::collections::HashMap;
use std::sync::OnceLock;

/// Full-width space then an arrow: the paywalled original, indented under the
/// free replacement that stands in for it.
pub const PAYWALL_CONT_PREFIX: &str = "　↳ ";
pub const PAYWALL_NOTE: &str = "⚠️ 付費牆（原文需訂閱）";

fn chars(s: &str) -> usize {
    s.chars().count()
}

fn take_chars(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// One numbered candidate as offered to the model.
#[derive(Debug, Clone, Default)]
pub struct Numbered {
    pub title: String,
    pub link: String,
    pub source_name: String,
}

impl From<&Item> for Numbered {
    fn from(i: &Item) -> Self {
        Self {
            title: i.title.clone(),
            link: i.link.clone(),
            source_name: i.source.clone(),
        }
    }
}

/// What a paywalled pick gets in place of a body the reader cannot open.
///
/// Three states, in descending order of what they give the reader: a free
/// article covering the same story (`title_zh` + `link`), a summary of the
/// paid article itself (`summary`), or neither — the headline plus the note.
/// The two are never both populated: a summary is only written for an entry
/// that found no replacement.
#[derive(Debug, Clone, Default)]
pub struct Replacement {
    pub title_zh: String,
    pub link: String,
    /// Summary lines of the paywalled article, already newline-separated.
    pub summary: String,
}

/// Title → link, in feed order.
///
/// Ordered, not a `HashMap`: the substring fallback in [`attach_links`] takes
/// the first entry that matches, and with several candidates that choice is
/// only reproducible if the scan order is. A hash map would pick a different
/// source for the same digest between runs.
#[derive(Debug, Default, Clone)]
pub struct LinkMap(Vec<(String, String)>);

impl LinkMap {
    pub fn get(&self, title: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(t, _)| t == title)
            .map(|(_, l)| l.as_str())
    }

    /// Last write wins for a repeated title, matching dict assignment.
    pub fn insert(&mut self, title: String, link: String) {
        match self.0.iter_mut().find(|(t, _)| *t == title) {
            Some(slot) => slot.1 = link,
            None => self.0.push((title, link)),
        }
    }

    fn first_substring_match(&self, text: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(t, _)| t.contains(text) || text.contains(t.as_str()))
            .map(|(_, l)| l.as_str())
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

pub fn build_link_map(all_items: &[(String, Vec<Item>)]) -> LinkMap {
    let mut map = LinkMap::default();
    for (_, items) in all_items {
        for it in items {
            let title = it.title.trim();
            if !title.is_empty() && !it.link.is_empty() {
                map.insert(title.to_string(), it.link.clone());
            }
        }
    }
    map
}

/// Attach links by matching the bullet text against a title.
///
/// Used only on the fallback path, where there are no `#N` markers to key on.
/// The substring fallback is deliberately loose, so a slightly reworded
/// headline still reaches its source; the first match in feed order wins.
pub fn attach_links(summary: &str, link_map: &LinkMap) -> String {
    summary
        .split('\n')
        .map(|line| {
            if !line.starts_with("- ") || line.contains("[🔗]") || line.contains("http") {
                return line.to_string();
            }
            let title_text = line[2..].trim();
            let link = link_map
                .get(title_text)
                .or_else(|| link_map.first_substring_match(title_text));
            let safe = neutralize_markdown(title_text);
            match link {
                Some(l) => format!("- {safe} [🔗]({l})"),
                None => format!("- {safe}"),
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn noise_marker_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"^\s*\[[^\]]*\]\s*$").expect("literal"))
}

fn linked(body: &str, link: &str, count: &mut usize) -> String {
    if !link.is_empty() {
        *count += 1;
        return format!("- {body} [🔗]({link})");
    }
    if body.is_empty() {
        "-".to_string()
    } else {
        format!("- {body}")
    }
}

/// A model-written headline with fewer CJK characters than this is a fragment,
/// not a headline — provided its source is long enough to have said more
/// (see [`DEGENERATE_SOURCE_MIN_CJK`]).
///
/// Both floors are absolute on purpose. Ratio checks were tried on paper and
/// rejected: a Chinese translation of an English title is legitimately about
/// 30% of its character length, and the model is asked to condense long CJK
/// titles, so any ratio low enough to spare those cases is too low to catch
/// anything. The case behind this is 2026-11-13, when 重大新聞 shipped
/// `- 川普喊` for a full headline.
pub const DEGENERATE_BODY_MAX_CJK: usize = 6;

/// A source title needs at least this many CJK ideographs before a short body
/// counts as degenerate. Below it, a short body may be a faithful short
/// headline. An English source never reaches it, so its short translation is
/// left alone.
///
/// Reaching it does **not** make the title Traditional Chinese: a Japanese or
/// Simplified Chinese headline clears it just as easily, and substituting one
/// would put the wrong script into the digest. [`is_traditional_chinese`]
/// decides whether a degenerate body may actually be replaced.
pub const DEGENERATE_SOURCE_MIN_CJK: usize = 10;

/// High-frequency Simplified-only characters, each with a distinct Traditional
/// form (們這個為國發時對說經會實現關進開動產東車門問間過還讓網電買賣業應華兩處
/// 務則層訊資級億價場長) and none in ordinary Traditional use.
///
/// A heuristic, not a converter: a short Simplified title that happens to use
/// none of them passes as Traditional. `会` and `国` are also Japanese
/// shinjitai, which is a bonus for a Japanese title written without kana.
const SIMPLIFIED_ONLY: &str = "们这个为国发时对说经会实现关进开动产东车门问间过还让网电买卖业应华两处务则层讯资级亿价场长";

/// Hiragana, katakana, the katakana extensions, and half-width katakana.
///
/// U+30FB (`・`) is excluded: Taiwanese and Hong Kong outlets use it to
/// separate transliterated names (唐納・川普), so it says nothing about the
/// language.
fn is_kana(c: char) -> bool {
    (('\u{3040}'..='\u{30FF}').contains(&c) && c != '\u{30FB}')
        || ('\u{31F0}'..='\u{31FF}').contains(&c)
        || ('\u{FF66}'..='\u{FF9F}').contains(&c)
}

/// Traditional forms that differ from **both** their Simplified form **and**
/// their Japanese standard (shinjitai) form, so a title containing one is
/// neither Simplified Chinese nor modern Japanese.
///
/// The rule for adding a character: its Simplified form and its Japanese form
/// must both be different code points. 國 (国/国), 發 (发/発), 說 (说/説) and
/// 產 (产/産) qualify; 開, 話, 談, 時, 門, 買, 進, 電 do not, because Japanese
/// writes them identically, and an all-kanji Japanese headline made of them
/// would pass.
const TRADITIONAL_ONLY: &str = "國會發對圖學經實關戰體讀賣氣廣歐區應當黨總舊萬與來說產讓兩處價灣臺參獎稅數歲號鐵證險據變舉權濟擊";

/// Whether a headline may stand in a Traditional Chinese digest as-is.
///
/// Fails closed: no kana, none of [`SIMPLIFIED_ONLY`], **and** at least one
/// of [`TRADITIONAL_ONLY`]. The first two alone are a blacklist, and a
/// Simplified title avoiding the listed characters (人工智能图像识别) or an
/// all-kanji Japanese one (半導体性能比較検証結果) passed it. The cost is a
/// Traditional title built only of shared characters, which is judged
/// uncertain and keeps the model's text — traced as `replaced: false`, never
/// replaced by something that might be the wrong script.
fn is_traditional_chinese(text: &str) -> bool {
    !text.chars().any(|c| is_kana(c) || SIMPLIFIED_ONLY.contains(c))
        && text.chars().any(|c| TRADITIONAL_ONLY.contains(c))
}

/// A model-written body judged degenerate against its source.
struct Degenerate<'a> {
    /// The source title with its ` - Source` suffix removed.
    source: &'a str,
    /// Whether `source` may be rendered in its place.
    replaceable: bool,
}

fn degenerate_check<'a>(written: &str, source_title: &'a str) -> Option<Degenerate<'a>> {
    let source = title_without_source(source_title.trim()).trim();
    let degenerate = count_cjk(written) < DEGENERATE_BODY_MAX_CJK
        && count_cjk(source) >= DEGENERATE_SOURCE_MIN_CJK;
    degenerate.then(|| Degenerate {
        source,
        replaceable: is_traditional_chinese(source),
    })
}

/// Render marked bullets into linked ones, dropping anything unrecognised.
///
/// Returns the rendered text and how many links were attached — the caller
/// treats zero as a failed translation, because a section with no links is not
/// a digest.
///
/// Everything that is not a marked bullet or known scaffolding is dropped. The
/// model occasionally leaks instrumentation like `[hint_picked:none]`, whose
/// stray underscores fail the Markdown safety probe below and force the whole
/// message into plaintext — which expands every `[🔗]` into a raw URL.
///
/// A degenerate body (see [`DEGENERATE_BODY_MAX_CJK`]) is replaced by its
/// source title before anything else uses it, so every paywall shape below
/// renders the repaired text — when that title is Traditional Chinese.
/// Otherwise the body ships as written, and either way the item is traced.
pub fn attach_numbered_links(
    summary: &str,
    numbered: &HashMap<u32, Numbered>,
    paywall: &HashMap<u32, Replacement>,
) -> (String, usize) {
    let mut attached = 0usize;
    let mut dropped_noise: Vec<String> = Vec::new();
    let mut dropped_unknown: Vec<String> = Vec::new();
    let mut degenerate: Vec<serde_json::Value> = Vec::new();
    let mut rendered: Vec<String> = Vec::new();

    for line in summary.lines() {
        let Some((num, digits_end)) = leading_marker_span(line) else {
            let stripped = line.trim();
            let keep = stripped.is_empty()
                || matches!(stripped, "---" | "--" | "***")
                || (stripped.starts_with("**") && stripped.ends_with("**"))
                || stripped.contains("今日無相關新聞")
                // The prefix opens with U+3000, which `trim` would remove, so
                // the raw line is checked first and the trimmed one only for
                // defence in depth.
                || line.starts_with(PAYWALL_CONT_PREFIX)
                || stripped.starts_with(PAYWALL_CONT_PREFIX);
            if keep {
                rendered.push(line.to_string());
            } else if noise_marker_re().is_match(line) {
                dropped_noise.push(stripped.to_string());
            } else {
                dropped_unknown.push(stripped.to_string());
            }
            continue;
        };

        let item = numbered.get(&num);
        let written = line[digits_end..].trim_start();
        let check = item.and_then(|i| degenerate_check(written, &i.title));
        if let Some(d) = &check {
            degenerate.push(json!({
                "n": num,
                "written": take_chars(written.trim(), 120),
                "source": take_chars(d.source, 120),
                "replaced": d.replaceable,
            }));
        }
        let repaired = check.filter(|d| d.replaceable).map(|d| d.source);
        let body = neutralize_markdown(repaired.unwrap_or(written));
        let link = item.map(|i| i.link.as_str()).unwrap_or("");

        match paywall.get(&num) {
            Some(rep) if !rep.title_zh.trim().is_empty() && !rep.link.trim().is_empty() => {
                let rep_title = neutralize_markdown(rep.title_zh.trim());
                // The free replacement leads; the paywalled original follows,
                // indented. `linked` runs once per rendered line so the counter
                // counts each link at most once.
                let orig = linked(&body, link, &mut attached);
                // Past the "- " that `linked` just wrote; a bodyless bullet
                // is the single char "-", and dropping two from it is empty.
                let orig_body: String = orig.chars().skip(2).collect();
                let orig_line =
                    format!("{PAYWALL_CONT_PREFIX}原文：{orig_body}  {PAYWALL_NOTE}");
                let head = linked(&rep_title, rep.link.trim(), &mut attached);
                rendered.push(format!("{head}\n{orig_line}"));
            }
            Some(rep) if !rep.summary.trim().is_empty() => {
                // Nothing free covers this story, but the article itself was
                // readable, so the reader gets what it said. The summary lines
                // carry the continuation prefix so the chunker keeps them with
                // the headline they belong to.
                let l = linked(&body, link, &mut attached);
                let mut block = format!("{l}  {PAYWALL_NOTE}");
                for s in rep.summary.lines().filter(|s| !s.trim().is_empty()) {
                    block.push_str(&format!(
                        "\n{PAYWALL_CONT_PREFIX}{}",
                        neutralize_markdown(s.trim())
                    ));
                }
                rendered.push(block);
            }
            Some(_) => {
                // Nothing free found and nothing readable — one bullet plus
                // the note, degraded.
                let l = linked(&body, link, &mut attached);
                rendered.push(format!("{l}  {PAYWALL_NOTE}"));
            }
            None => rendered.push(linked(&body, link, &mut attached)),
        }
    }

    if !degenerate.is_empty() {
        log_trace(
            "headline_degenerate",
            json!({"count": degenerate.len(), "items": degenerate}),
        );
    }
    if !dropped_noise.is_empty() {
        log_trace(
            "section_noise_dropped",
            json!({
                "count": dropped_noise.len(),
                "samples": dropped_noise.iter().take(5)
                    .map(|s| take_chars(s, 120)).collect::<Vec<_>>()
            }),
        );
    }
    if !dropped_unknown.is_empty() {
        log_trace(
            "section_unknown_line_dropped",
            json!({
                "count": dropped_unknown.len(),
                "samples": dropped_unknown.iter().take(5)
                    .map(|s| take_chars(s, 120)).collect::<Vec<_>>()
            }),
        );
    }
    (rendered.join("\n"), attached)
}

// ── length management ────────────────────────────────────────────────────────

fn link_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"\s*\[🔗\]\([^)]+\)\s*").expect("literal"))
}

fn bullet_dash_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?m)^-\s*").expect("literal"))
}

fn md_link_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"\[([^\]]+)\]\([^)]+\)").expect("literal"))
}

/// Remove `[🔗](url)` while keeping the line readable.
pub fn strip_links_keep_spacing(value: &str) -> String {
    let v = link_re().replace_all(value, " ");
    let v = bullet_dash_re().replace_all(&v, "- ");
    v.lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
}

/// What the reader actually sees: link labels without their URLs.
///
/// Telegram's limit applies after entity parsing, so this — not the raw
/// Markdown — is the length that matters for whether links can be kept.
pub fn markdown_visible_text(text: &str) -> String {
    md_link_re().replace_all(text, "$1").into_owned()
}

/// Drop links from the bottom up until the text fits.
pub fn trim_links_to_limit(text: &str, limit: usize) -> String {
    if chars(text) <= limit {
        return text.to_string();
    }
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    for idx in (0..lines.len()).rev() {
        if !lines[idx].contains("[🔗](") {
            continue;
        }
        lines[idx] = strip_links_keep_spacing(&lines[idx]);
        let candidate = lines.join("\n");
        if chars(&candidate) <= limit {
            return candidate;
        }
    }
    trim_lines_to_limit(&strip_links_keep_spacing(text), limit)
}

/// Drop whole bullets from the bottom up until the text fits.
pub fn trim_lines_to_limit(text: &str, limit: usize) -> String {
    if chars(text) <= limit {
        return text.to_string();
    }
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    for idx in (0..lines.len()).rev() {
        if !lines[idx].trim_start().starts_with('-') {
            continue;
        }
        // Take any paywall continuation with the bullet, so a pair is never
        // left as an orphaned 原文 note with no headline above it.
        let mut end = idx + 1;
        while end < lines.len() && lines[end].starts_with(PAYWALL_CONT_PREFIX) {
            end += 1;
        }
        lines.drain(idx..end);
        let candidate = lines.join("\n");
        if chars(&candidate) <= limit {
            return candidate;
        }
    }

    if limit <= 20 {
        return take_chars(text, limit);
    }
    format!(
        "{}\n…（已截短）",
        take_chars(text, limit - 20).trim_end()
    )
}

/// Keep source links while the visible digest is short enough; otherwise drop
/// them outside the AI section first, since that is the section a reader is
/// most likely to follow through on.
pub fn trim_digest_links(text: &str) -> String {
    if chars(&markdown_visible_text(text)) <= 4000 {
        return text.to_string();
    }
    let mut in_ai = false;
    let trimmed: Vec<String> = text
        .split('\n')
        .map(|line| {
            if line.contains("AI 人工智慧") {
                in_ai = true;
            } else if line.starts_with("**") {
                in_ai = false;
            }
            if !in_ai && line.contains("[🔗](") {
                strip_links_keep_spacing(line)
            } else {
                line.to_string()
            }
        })
        .collect();
    let result = trimmed.join("\n");
    if chars(&result) <= 4000 {
        return result;
    }
    trim_links_to_limit(text, 4000)
}

/// Split into chunks on line boundaries, keeping a paywall pair together.
pub fn split_message_preserving_lines(body: &str, limit: usize) -> Vec<String> {
    if chars(body) <= limit {
        return vec![body.to_string()];
    }

    let mut chunks: Vec<String> = Vec::new();
    let mut current: Vec<String> = Vec::new();
    let mut current_len = 0usize;

    let flush = |chunks: &mut Vec<String>, current: &mut Vec<String>, current_len: &mut usize| {
        if !current.is_empty() {
            chunks.push(current.concat().trim_end().to_string());
            current.clear();
            *current_len = 0;
        }
    };

    let raw_lines = lines_keepends(body);
    for (idx, line) in raw_lines.iter().enumerate() {
        let this_is_cont = line.trim_start_matches('\n').starts_with(PAYWALL_CONT_PREFIX);
        // A continuation must never open a chunk without its parent bullet, so
        // a flush may happen before the parent but never between the pair.
        let next_is_cont = raw_lines
            .get(idx + 1)
            .is_some_and(|n| n.trim_start_matches('\n').starts_with(PAYWALL_CONT_PREFIX));
        let line_len = chars(line);

        if line_len > limit {
            if this_is_cont && !current.is_empty() {
                // An over-long continuation stays with the chunk holding its
                // parent rather than opening a fresh one.
                current.push(line.clone());
                current_len += line_len;
                continue;
            }
            flush(&mut chunks, &mut current, &mut current_len);
            let pieces: Vec<String> = chunk_chars(line, limit)
                .into_iter()
                .map(|p| p.trim_end().to_string())
                .collect();
            if next_is_cont && !pieces.is_empty() {
                // Resume on the last piece so the continuation joins it. The
                // combined tail may exceed the limit; a truncated pair is worse.
                for p in &pieces[..pieces.len() - 1] {
                    chunks.push(p.clone());
                }
                current = vec![format!("{}\n", pieces[pieces.len() - 1])];
                current_len = chars(&current[0]);
                continue;
            }
            chunks.extend(pieces);
            continue;
        }

        if !current.is_empty() && current_len + line_len > limit && !this_is_cont {
            flush(&mut chunks, &mut current, &mut current_len);
        }
        current.push(line.clone());
        current_len += line_len;
    }

    flush(&mut chunks, &mut current, &mut current_len);
    chunks.into_iter().filter(|c| !c.is_empty()).collect()
}

/// `str.splitlines(keepends=True)` — the terminator stays on the line.
///
/// Only `\n` is treated as a break. Python also splits on `\r`, `\x0b`,
/// ` ` and friends; a headline containing one of those would chunk
/// differently, which is noted rather than reproduced because the extra
/// separators have no meaning in a Telegram message.
fn lines_keepends(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for ch in s.chars() {
        cur.push(ch);
        if ch == '\n' {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn chunk_chars(s: &str, limit: usize) -> Vec<String> {
    let cs: Vec<char> = s.chars().collect();
    cs.chunks(limit).map(|c| c.iter().collect()).collect()
}

/// Would Telegram's legacy Markdown parser reject this chunk?
///
/// Link markup is skipped wholesale — a URL may contain any of the emphasis
/// characters without them being emphasis — and what remains must have an even
/// count of each, ignoring backslash escapes.
pub fn markdown_chunk_is_safe(chunk: &str) -> Result<(), &'static str> {
    let cs: Vec<char> = chunk.chars().collect();
    let mut probe: Vec<char> = Vec::new();
    let mut i = 0;
    while i < cs.len() {
        if cs[i] == '[' {
            let close_label = cs[i + 1..].iter().position(|c| *c == ']').map(|p| p + i + 1);
            let Some(close_label) = close_label else {
                return Err("unclosed link bracket");
            };
            if cs.get(close_label + 1) == Some(&'(') {
                let close_url = cs[close_label + 2..]
                    .iter()
                    .position(|c| *c == ')')
                    .map(|p| p + close_label + 2);
                let Some(close_url) = close_url else {
                    return Err("unclosed link url");
                };
                i = close_url + 1;
                continue;
            }
        }
        probe.push(cs[i]);
        i += 1;
    }

    if chunk.ends_with('\\') && !chunk.ends_with("\\\\") {
        return Err("trailing backslash");
    }

    for (marker, name) in [
        ('*', "unmatched asterisk"),
        ('_', "unmatched underscore"),
        ('`', "unmatched backtick"),
    ] {
        let mut count = 0;
        let mut escaped = false;
        for ch in &probe {
            if escaped {
                escaped = false;
                continue;
            }
            match ch {
                '\\' => escaped = true,
                c if *c == marker => count += 1,
                _ => {}
            }
        }
        if count % 2 != 0 {
            return Err(name);
        }
    }
    Ok(())
}

pub fn telegram_chunks(body: &str) -> Vec<String> {
    split_message_preserving_lines(body, TELEGRAM_RAW_CHUNK_LIMIT)
}

#[cfg(test)]
mod script_sets {
    use super::{is_kana, SIMPLIFIED_ONLY, TRADITIONAL_ONLY};
    use crate::validate::count_cjk;
    use std::collections::HashSet;

    #[test]
    fn the_marker_sets_are_disjoint() {
        let simplified: HashSet<char> = SIMPLIFIED_ONLY.chars().collect();
        let both: Vec<char> = TRADITIONAL_ONLY.chars().filter(|c| simplified.contains(c)).collect();
        assert!(both.is_empty(), "{both:?}");
    }

    #[test]
    fn no_traditional_marker_is_kana() {
        let kana: Vec<char> = TRADITIONAL_ONLY.chars().filter(|c| is_kana(*c)).collect();
        assert!(kana.is_empty(), "{kana:?}");
    }

    #[test]
    fn every_marker_is_a_single_listed_ideograph() {
        for set in [SIMPLIFIED_ONLY, TRADITIONAL_ONLY] {
            let n = set.chars().count();
            assert_eq!(count_cjk(set), n, "{set}");
            assert_eq!(set.chars().collect::<HashSet<_>>().len(), n, "duplicate in {set}");
        }
    }

    #[test]
    fn characters_japanese_writes_identically_are_not_traditional_markers() {
        // A sample of the exclusions the rule names; adding any of them would
        // let an all-kanji Japanese headline through.
        for c in "開話談時門買進電業間過動東車則場長".chars() {
            assert!(!TRADITIONAL_ONLY.contains(c), "{c}");
        }
    }
}
