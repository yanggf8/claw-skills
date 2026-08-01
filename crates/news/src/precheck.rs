//! Checking the model's picks against the real articles.
//!
//! Tier 1 runs before the model sees anything and touches no network. Tier 2
//! runs on what the model chose, while `#N` identity still exists, and is
//! bounded by a wall-clock deadline: an item still undecided when the deadline
//! passes is kept, because failing open costs a possible dud bullet while
//! failing closed costs a real story.

use crate::config::{
    paywall_replace_deadline, paywall_replace_enabled, paywall_replace_max,
    paywall_replace_sources, precheck_decode_timeout, precheck_enabled, precheck_fetch_timeout,
    precheck_max_workers, precheck_total_deadline,
};
use crate::feed::{
    bing_news_feed_url, fetch_feed, normalize_replacement_candidate, split_url, topic_feed_url,
};
use crate::quality::{self, Action, Verdict};
use crate::render::Replacement;
use crate::select::NumberedMap;
use crate::text::{dedup, title_without_source, topic_words, Item};
use crate::trace::log_trace;
use crate::validate::{leading_marker, leading_marker_ids};
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

/// For each paywalled pick, look for a free article on a *different* publisher
/// covering the same story, and translate its headline.
///
/// Bounded twice over: by a wall-clock deadline checked before every network
/// or model step, and by a cap on how many entries are attempted at all.
/// Without a replacement the render stage degrades to one bullet plus the
/// 付費牆 note, which is a worse digest but never a late one.
pub fn resolve_paywall_replacements(
    paywall: &mut PaywallMap,
    date_str: &str,
    translate: &dyn Fn(&str, &str) -> Option<String>,
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
        let orig_words = topic_words(&orig_title);

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
            // Same story, roughly: two shared significant tokens.
            if orig_words.intersection(&topic_words(&cand.title)).count() < 2 {
                continue;
            }
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
            let Some(title_zh) = translate(&cand.title, date_str) else {
                continue;
            };
            if title_zh.is_empty() {
                continue;
            }
            if let Some(entry) = paywall.get_mut(&num) {
                entry.replacement = Some(Replacement {
                    title_zh,
                    link: verdict.decoded_url.clone().unwrap_or(cand.link.clone()),
                });
            }
            log_trace(
                "paywall_replacement_found",
                json!({"marker": num, "source": cand.source}),
            );
            break;
        }
    }
}

/// The shape the renderer wants: only entries that actually found something.
pub fn render_replacements(paywall: &PaywallMap) -> HashMap<u32, Replacement> {
    paywall
        .iter()
        .map(|(k, v)| {
            (
                *k,
                v.replacement.clone().unwrap_or_default(),
            )
        })
        .collect()
}
