//! Checking the model's picks against the real articles.
//!
//! Tier 1 runs before the model sees anything and touches no network. Tier 2
//! runs on what the model chose, while `#N` identity still exists, and is
//! bounded by a wall-clock deadline: an item still undecided when the deadline
//! passes is kept, because failing open costs a possible dud bullet while
//! failing closed costs a real story.

use crate::config::{
    paywall_replace_cross_lang, paywall_replace_deadline, paywall_replace_enabled,
    paywall_replace_max, paywall_replace_sources, paywall_summary_body_chars,
    paywall_summary_enabled, paywall_summary_min_words, precheck_decode_timeout, precheck_enabled,
    precheck_fetch_timeout, precheck_max_workers, precheck_total_deadline,
};
use crate::feed::{
    bing_news_feed_url, fetch_feed, normalize_replacement_candidate, split_url, topic_feed_url,
};
use crate::quality::{self, Action, Verdict};
use crate::render::Replacement;
use crate::select::NumberedMap;
use crate::text::{dedup, title_without_source, topic_words, Item};
use crate::trace::log_trace;
use crate::validate::{count_cjk, leading_marker, leading_marker_ids};
use claw_core::budget::monotonic_secs;
use serde_json::json;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Memo shared by every precheck call in one run, keyed by link, so the AI
/// Level-3 re-subdivision — which re-covers items Level-2 already saw — does
/// not decode or fetch them twice.
pub type SharedCache = Arc<Mutex<HashMap<String, Verdict>>>;

pub fn new_cache() -> SharedCache {
    Arc::new(Mutex::new(HashMap::new()))
}

/// Look up, compute outside the lock, then record.
///
/// The network call deliberately happens with no lock held, so two workers can
/// race to resolve the same link. That wastes one fetch and is what the Python
/// does; holding the lock across the fetch would serialise the whole pool.
fn cached_precheck(
    cache: &SharedCache,
    source_name: &str,
    title: &str,
    link: &str,
    decoded_hint: Option<&str>,
) -> Verdict {
    let hinted = decoded_hint.is_some_and(|s| !s.is_empty());
    if !hinted {
        if let Some(hit) = cache.lock().ok().and_then(|c| c.get(link).cloned()) {
            return hit;
        }
    }
    let verdict = quality::precheck_action(
        source_name,
        title,
        link,
        decoded_hint,
        Duration::from_secs_f64(precheck_decode_timeout()),
        Duration::from_secs_f64(precheck_fetch_timeout()),
        None,
    );
    if !link.is_empty() {
        if let Ok(mut c) = cache.lock() {
            c.insert(link.to_string(), verdict.clone());
        }
    }
    verdict
}

/// Tier 1: drop deny-listed sources, nothing else.
///
/// Promo judgement is left to the prompt and to Tier 2's title-only check.
/// Titles are not gated here and promo is never matched against body text —
/// bare substrings like 限時 over-match real news such as 限時降息.
pub fn tier1_filter_items(items: Vec<Item>) -> Vec<Item> {
    if !precheck_enabled() || items.is_empty() {
        return items;
    }
    let deny = &quality::active_config().deny;
    let before = items.len();
    let kept: Vec<Item> = items
        .into_iter()
        .filter(|it| !deny.contains(&it.source))
        .collect();
    log_trace(
        "quality_tier1",
        json!({"before": before, "after": kept.len(), "dropped": before - kept.len()}),
    );
    kept
}

/// What Tier 2 recorded about one paywalled pick.
#[derive(Debug, Clone, Default)]
pub struct PaywallEntry {
    pub decoded_url: Option<String>,
    pub reason: Option<String>,
    pub title: String,
    pub source_name: String,
    /// Filled in later by [`resolve_paywall_replacements`], if a free article
    /// covering the same story turns up.
    pub replacement: Option<Replacement>,
    /// A summary of the paywalled article itself, written only when no
    /// replacement was found — see [`resolve_paywall_summaries`].
    pub summary: Option<String>,
}

pub type PaywallMap = BTreeMap<u32, PaywallEntry>;

/// Tier 2: body-precheck the selected items before links are attached.
///
/// `drop` removes the bullet. `title_only` keeps the model's bullet **exactly
/// as written** and records the item so the render stage can add a free
/// replacement and a 付費牆 note — the digest is Traditional Chinese and a
/// language gate enforces that, so rewriting a bullet back to the raw RSS
/// headline would inject English or Japanese past the gate. `keep` does
/// nothing.
pub fn precheck_apply(
    summary: &str,
    numbered: &NumberedMap,
    section: &str,
    cache: &SharedCache,
) -> (String, PaywallMap) {
    if !precheck_enabled() || summary.trim().is_empty() {
        return (summary.to_string(), PaywallMap::new());
    }
    let known: HashSet<u32> = numbered.keys().copied().collect();
    let selected = leading_marker_ids(summary, &known);
    if selected.is_empty() {
        return (summary.to_string(), PaywallMap::new());
    }

    let (tx, rx) = mpsc::channel::<(u32, Verdict)>();
    let queue: Arc<Mutex<Vec<u32>>> = Arc::new(Mutex::new(selected.iter().rev().copied().collect()));
    let workers = precheck_max_workers().min(selected.len()).max(1);

    for _ in 0..workers {
        let queue = Arc::clone(&queue);
        let cache = Arc::clone(cache);
        let tx = tx.clone();
        let items: Vec<(u32, crate::render::Numbered)> = selected
            .iter()
            .filter_map(|n| numbered.get(n).map(|v| (*n, v.clone())))
            .collect();
        std::thread::spawn(move || loop {
            let Some(num) = queue.lock().ok().and_then(|mut q| q.pop()) else {
                return;
            };
            let item = items.iter().find(|(n, _)| *n == num).map(|(_, v)| v.clone());
            let item = item.unwrap_or_default();
            let verdict = cached_precheck(&cache, &item.source_name, &item.title, &item.link, None);
            // A closed receiver means the deadline passed; stop rather than
            // keep fetching for results nobody will read.
            if tx.send((num, verdict)).is_err() {
                return;
            }
        });
    }
    drop(tx);

    let mut actions: BTreeMap<u32, Action> = BTreeMap::new();
    let mut verdicts: BTreeMap<u32, Verdict> = BTreeMap::new();
    let deadline = std::time::Instant::now() + Duration::from_secs_f64(precheck_total_deadline());
    while actions.len() < selected.len() {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            eprintln!("[WARN: tier2 precheck deadline hit: section={section}]");
            break;
        }
        match rx.recv_timeout(left) {
            Ok((num, verdict)) => {
                actions.insert(num, verdict.action);
                verdicts.insert(num, verdict);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                eprintln!("[WARN: tier2 precheck deadline hit: section={section}]");
                break;
            }
            // Every worker finished; whatever is missing had no numbered entry.
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    // Stragglers are abandoned rather than joined: the deadline has to bound
    // wall clock, and a worker already inside a socket read cannot be
    // interrupted anyway.
    drop(rx);

    let n_drop = actions.values().filter(|a| **a == Action::Drop).count();
    let n_title = actions.values().filter(|a| **a == Action::TitleOnly).count();
    log_trace(
        "quality_tier2",
        json!({"section": section, "checked": selected.len(),
               "dropped": n_drop, "paywalled_kept": n_title}),
    );

    let mut paywall = PaywallMap::new();
    for (num, action) in &actions {
        if *action != Action::TitleOnly {
            continue;
        }
        let item = numbered.get(num).cloned().unwrap_or_default();
        let v = verdicts.get(num);
        paywall.insert(
            *num,
            PaywallEntry {
                decoded_url: v.and_then(|v| v.decoded_url.clone()),
                reason: v.and_then(|v| v.reason.map(str::to_string)),
                title: item.title,
                source_name: item.source_name,
                replacement: None,
                summary: None,
            },
        );
    }

    if n_drop == 0 {
        return (summary.to_string(), paywall);
    }

    let out: Vec<&str> = summary
        .lines()
        .filter(|line| match leading_marker(line) {
            Some(n) => actions.get(&n) != Some(&Action::Drop),
            None => true,
        })
        .collect();
    (out.join("\n"), paywall)
}

/// Two hosts sharing a registrable domain, taken as the last two labels.
///
/// A deliberately simple heuristic — enough to reject a same-publisher "free"
/// candidate without pulling in a public-suffix list. It over-merges under
/// suffixes like `co.uk`, which costs a missed replacement rather than a wrong
/// one.
pub fn same_registered_domain(host_a: &str, host_b: &str) -> bool {
    fn reg(host: &str) -> String {
        let h = host.to_lowercase();
        let h = h.trim();
        let h = h.rsplit('@').next().unwrap_or(h);
        let h = h.split(':').next().unwrap_or(h);
        let h = h.trim_matches('.');
        let parts: Vec<&str> = h.split('.').collect();
        if parts.len() >= 2 {
            parts[parts.len() - 2..].join(".")
        } else {
            parts.join(".")
        }
    }
    let (a, b) = (reg(host_a), reg(host_b));
    !a.is_empty() && a == b
}

fn netloc_lower(url: &str) -> String {
    split_url(url)
        .map(|(h, _, _)| h.to_lowercase())
        .unwrap_or_default()
}

/// Whether a headline is written in CJK, judged after the source suffix is
/// removed.
///
/// The suffix matters: Google appends `" - 自由時報"` to an English headline
/// carried by a Taiwanese outlet, and counting that would call the headline
/// Chinese when it is not.
fn is_cjk_title(title: &str) -> bool {
    count_cjk(title_without_source(title)) >= 2
}

/// The three answers the deterministic same-story check can give.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoryGate {
    /// Enough shared tokens to treat the two headlines as one event.
    Same,
    /// Same script, too few shared tokens.
    Different,
    /// Different scripts. Token overlap cannot answer this — see
    /// [`same_story_gate`].
    Undecidable,
}

/// Does this candidate cover the same story, as far as tokens can tell?
///
/// Two shared significant tokens mean "same event". That test is sound only
/// while both headlines are in one script: `topic_words` emits Latin runs for
/// English and CJK *bigrams* for Chinese, and no Latin run can ever equal a
/// bigram. So an English original and a Chinese candidate intersect in exactly
/// zero tokens no matter how plainly they report the same thing — the count is
/// a property of the alphabets, not of the stories.
///
/// Observed 2026-08-12: Wired's "A New Trick Reveals AI Models' Inner Thoughts"
/// against TechNews' "AI「內心戲」全曝光？新技術破解大模型推理軌跡…" — the
/// TechNews piece cites the Wired one as its source, and the intersection was
/// still 0. Even `AI` cannot bridge it, being two characters where
/// `topic_words` keeps Latin runs of three or more.
///
/// So a cross-script pair is reported as [`StoryGate::Undecidable`] rather than
/// rejected, and the caller decides it by other means.
pub fn same_story_gate(orig_title: &str, cand_title: &str) -> StoryGate {
    if is_cjk_title(orig_title) != is_cjk_title(cand_title) {
        return StoryGate::Undecidable;
    }
    if topic_words(orig_title)
        .intersection(&topic_words(cand_title))
        .count()
        >= 2
    {
        StoryGate::Same
    } else {
        StoryGate::Different
    }
}

/// The Traditional Chinese headline to show for an accepted candidate, or
/// `None` to reject it.
///
/// The two model paths are alternatives, never both: a same-script candidate is
/// translated, and a cross-script one is settled by `judge`, which answers the
/// equivalence question and returns the headline in one call. That keeps the
/// cost of a cross-language replacement at exactly one model call — the same as
/// the translation it replaces — so the pass's wall-clock budget is unchanged.
pub fn headline_for(
    gate: StoryGate,
    orig_title: &str,
    cand_title: &str,
    date_str: &str,
    translate: &dyn Fn(&str, &str) -> Option<String>,
    judge: &dyn Fn(&str, &str, &str) -> Option<String>,
) -> Option<String> {
    let title = match gate {
        StoryGate::Different => return None,
        StoryGate::Same => translate(cand_title, date_str)?,
        StoryGate::Undecidable => judge(orig_title, cand_title, date_str)?,
    };
    (!title.trim().is_empty()).then_some(title)
}

/// For each paywalled pick, look for a free article on a *different* publisher
/// covering the same story, and translate its headline.
///
/// Bounded twice over: by a wall-clock deadline checked before every network
/// or model step, and by a cap on how many entries are attempted at all.
/// Without a replacement the render stage degrades to one bullet plus the
/// 付費牆 note, which is a worse digest but never a late one.
///
/// `judge` settles the cross-language case that `translate` cannot: see
/// [`same_story_gate`] for why token overlap is blind to it.
pub fn resolve_paywall_replacements(
    paywall: &mut PaywallMap,
    date_str: &str,
    translate: &dyn Fn(&str, &str) -> Option<String>,
    judge: &dyn Fn(&str, &str, &str) -> Option<String>,
) {
    if paywall.is_empty() || !paywall_replace_enabled() || !precheck_enabled() {
        return;
    }
    // Private to this pass. The main precheck memo is keyed by link alone, but
    // a candidate can share a link with a main-precheck item while carrying
    // different title and source metadata, and would inherit its verdict.
    let rep_cache = new_cache();
    let sources = paywall_replace_sources();
    let deadline = monotonic_secs() + paywall_replace_deadline();
    let expired = || monotonic_secs() >= deadline;
    let fetch_timeout = || -> Option<Duration> {
        if expired() {
            return None;
        }
        let left = (deadline - monotonic_secs()).clamp(0.5, 15.0);
        Some(Duration::from_secs_f64(left))
    };

    let mut processed = 0usize;
    let ids: Vec<u32> = paywall.keys().copied().collect();
    for num in ids {
        if processed >= paywall_replace_max() {
            break;
        }
        if expired() {
            log_trace("paywall_replace_deadline", json!({"resolved": processed}));
            break;
        }
        processed += 1;

        let (orig_title, orig_host) = {
            let entry = &paywall[&num];
            (
                entry.title.clone(),
                entry
                    .decoded_url
                    .as_deref()
                    .map(netloc_lower)
                    .unwrap_or_default(),
            )
        };
        if orig_title.trim().is_empty() {
            continue;
        }
        let query = title_without_source(&orig_title).trim().to_string();
        if query.is_empty() {
            continue;
        }
        if expired() {
            break;
        }

        let mut candidates: Vec<Item> = Vec::new();
        if sources.iter().any(|s| s == "google") {
            if let Some(t) = fetch_timeout() {
                candidates.extend(fetch_feed(&topic_feed_url(&query), 8, t));
            }
        }
        if sources.iter().any(|s| s == "bing") {
            if let Some(t) = fetch_timeout() {
                candidates.extend(
                    fetch_feed(&bing_news_feed_url(&query), 8, t)
                        .into_iter()
                        .map(normalize_replacement_candidate),
                );
            }
        }
        let candidates = dedup(&candidates);
        let cross_lang = paywall_replace_cross_lang();

        for cand in candidates {
            // Checked before each step, not just between entries, so one slow
            // entry cannot spend the whole budget.
            if expired() {
                log_trace("paywall_replace_deadline", json!({"resolved": processed}));
                break;
            }
            if cand.title.is_empty() || cand.link.is_empty() {
                continue;
            }
            // Cheap reject first, so an unrelated candidate never costs a
            // network precheck. An undecidable pair survives to be settled by
            // the model, but only after the cheap checks below have passed.
            let gate = match same_story_gate(&orig_title, &cand.title) {
                StoryGate::Different => continue,
                StoryGate::Undecidable if !cross_lang => continue,
                g => g,
            };
            let verdict = cached_precheck(
                &rep_cache,
                &cand.source,
                &cand.title,
                &cand.link,
                cand.decoded_url.as_deref(),
            );
            if matches!(verdict.action, Action::TitleOnly | Action::Drop) {
                continue; // the candidate is itself paywalled, or junk
            }
            // A resolved and *different* publisher is required. An unresolved
            // host cannot be confirmed distinct from the paywalled source, so
            // it is skipped rather than risk offering the same publisher's
            // article as the free alternative.
            let cand_host = verdict
                .decoded_url
                .as_deref()
                .map(netloc_lower)
                .unwrap_or_default();
            if cand_host.is_empty() {
                continue;
            }
            if !orig_host.is_empty() && same_registered_domain(&cand_host, &orig_host) {
                continue;
            }
            if expired() {
                log_trace("paywall_replace_deadline", json!({"resolved": processed}));
                break;
            }
            let Some(title_zh) = headline_for(
                gate,
                &orig_title,
                &cand.title,
                date_str,
                translate,
                judge,
            ) else {
                continue;
            };
            if let Some(entry) = paywall.get_mut(&num) {
                entry.replacement = Some(Replacement {
                    title_zh,
                    link: verdict.decoded_url.clone().unwrap_or(cand.link.clone()),
                    summary: String::new(),
                });
            }
            log_trace(
                "paywall_replacement_found",
                json!({"marker": num, "source": cand.source,
                       "cross_lang": gate == StoryGate::Undecidable}),
            );
            break;
        }
    }
}

/// Is this fetched text an article, or the chrome around a wall?
///
/// A metered paywall serves a crawler the whole piece — the wall is applied to
/// a browser, by a counter the fetcher never trips — so there is real text to
/// summarise. A hard paywall serves a stub, and the word count is what tells
/// the two apart.
pub fn body_is_summarisable(word_count: usize) -> bool {
    word_count >= paywall_summary_min_words()
}

/// Summarise the paywalled articles that no free replacement covered.
///
/// Runs after [`resolve_paywall_replacements`] and deliberately only on what it
/// left behind: a story with a free version is better served by a link to that
/// version than by this repo restating the paid one.
///
/// This re-fetches rather than reusing the precheck's body, because for a
/// listed paywall host there is no such body — `quality::precheck_action`
/// returns `TitleOnly` on the host check alone and never calls the fetcher. It
/// is the same reason the fetch is worth doing here: what the host list knows
/// is that a *reader* will be stopped, which says nothing about whether a
/// crawler was.
pub fn resolve_paywall_summaries(
    paywall: &mut PaywallMap,
    date_str: &str,
    summarise: &dyn Fn(&str, &str, &str) -> Option<String>,
) {
    if paywall.is_empty() || !paywall_summary_enabled() || !precheck_enabled() {
        return;
    }
    let deadline = monotonic_secs() + paywall_replace_deadline();
    let body_chars = paywall_summary_body_chars();

    for (num, entry) in paywall.iter_mut() {
        if entry.replacement.is_some() {
            continue;
        }
        if monotonic_secs() >= deadline {
            log_trace("paywall_summary_deadline", json!({"marker": num}));
            break;
        }
        let Some(url) = entry.decoded_url.clone().filter(|u| !u.is_empty()) else {
            continue;
        };
        let left = (deadline - monotonic_secs()).clamp(0.5, 15.0);
        let article = quality::fetch_article_text(&url, Duration::from_secs_f64(left));
        if article.error.is_some() || !body_is_summarisable(article.word_count) {
            log_trace(
                "paywall_summary_skipped",
                json!({"marker": num, "words": article.word_count,
                       "error": article.error.is_some()}),
            );
            continue;
        }
        let body: String = article.text.chars().take(body_chars).collect();
        entry.summary = summarise(&entry.title, &body, date_str).filter(|s| !s.trim().is_empty());
        log_trace(
            "paywall_summary",
            json!({"marker": num, "words": article.word_count,
                   "written": entry.summary.is_some()}),
        );
    }
}

/// The shape the renderer wants: only entries that actually found something.
pub fn render_replacements(paywall: &PaywallMap) -> HashMap<u32, Replacement> {
    paywall
        .iter()
        .map(|(k, v)| {
            let mut r = v.replacement.clone().unwrap_or_default();
            // Only when nothing free stood in: the renderer picks the
            // replacement branch first, and a summary alongside it would be
            // dead weight carried through the width and chunk guards.
            if r.title_zh.trim().is_empty() || r.link.trim().is_empty() {
                r.summary = v.summary.clone().unwrap_or_default();
            }
            (*k, r)
        })
        .collect()
}
