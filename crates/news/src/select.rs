//! Numbering candidates for the model, and cleaning up what it picks.
//!
//! Two dedup passes bracket the LLM call. Before it, high-overlap pairs are
//! offered as *hints* — advisory only, because a headline pair that shares
//! four tokens is usually but not always the same event. After it, the
//! selected set is hard-collapsed, because by then the model has committed and
//! two bullets about one event are simply a duplicate in the reader's digest.

use crate::config::{
    llm_post_dedup_enabled, LLM_DEDUP_HINT_OVERLAP, LLM_POST_DEDUP_OVERLAP,
};
use crate::render::Numbered;
use crate::text::{topic_words, Item};
use crate::trace::log_trace;
use crate::validate::{leading_marker, leading_marker_ids, news_bullet_lines, neutralize_markdown};
use serde_json::json;
use std::collections::{BTreeMap, HashSet};

/// Candidates keyed by the `#N` the model sees. A `BTreeMap` because several
/// steps iterate in id order and depend on it.
pub type NumberedMap = BTreeMap<u32, Numbered>;

/// Number the items and render the prompt's candidate block.
///
/// `limits` caps each label so a busy feed cannot make the prompt — and with
/// it the model's latency — grow without bound.
pub fn number_items_for_prompt(
    all_items: &[(String, Vec<Item>)],
    labels: Option<&[String]>,
    limits: &dyn Fn(&str) -> Option<usize>,
) -> (NumberedMap, String) {
    let mut numbered = NumberedMap::new();
    let mut sections: Vec<String> = Vec::new();
    let mut idx = 1u32;

    let order: Vec<String> = match labels {
        Some(l) => l.to_vec(),
        None => all_items.iter().map(|(k, _)| k.clone()).collect(),
    };

    for label in order {
        let Some((_, items)) = all_items.iter().find(|(k, _)| *k == label) else {
            continue;
        };
        if items.is_empty() {
            continue;
        }
        let limit = limits(&label).unwrap_or(items.len());
        let mut lines: Vec<String> = Vec::new();
        for it in items.iter().take(limit) {
            numbered.insert(idx, Numbered::from(it));
            lines.push(format!("  #{idx} {}", it.title));
            idx += 1;
        }
        sections.push(format!("[{label}]\n{}", lines.join("\n")));
    }
    (numbered, sections.join("\n"))
}

/// Independent high-overlap pairs, as `(a, b, overlap)` with `a < b`.
///
/// Independent means no transitive closure: A–B and B–C are two hints, not one
/// group of three. Ids are sorted first so the list is the same on every run.
pub fn dedup_pair_hints(numbered: &NumberedMap, min_overlap: usize) -> Vec<(u32, u32, usize)> {
    let ids: Vec<u32> = numbered.keys().copied().collect();
    let word_sets: Vec<HashSet<String>> = ids
        .iter()
        .map(|i| topic_words(&numbered[i].title))
        .collect();

    let mut pairs = Vec::new();
    for (i, a) in ids.iter().enumerate() {
        if word_sets[i].is_empty() {
            continue;
        }
        for (j, b) in ids.iter().enumerate().skip(i + 1) {
            if word_sets[j].is_empty() {
                continue;
            }
            let overlap = word_sets[i].intersection(&word_sets[j]).count();
            if overlap >= min_overlap {
                pairs.push((*a, *b, overlap));
            }
        }
    }
    pairs
}

pub fn default_pair_hints(numbered: &NumberedMap) -> Vec<(u32, u32, usize)> {
    dedup_pair_hints(numbered, LLM_DEDUP_HINT_OVERLAP)
}

/// Render the hints for the prompt. Empty when there are none, so the prompt
/// does not carry a heading with nothing under it.
pub fn format_dedup_hint_block(pairs: &[(u32, u32, usize)]) -> String {
    if pairs.is_empty() {
        return String::new();
    }
    let parts: Vec<String> = pairs.iter().map(|(a, b, _)| format!("#{a}+#{b}")).collect();
    format!(
        "可能同事件候選（僅供複核，仍按事件語義判斷；非硬性合併）：{}\n\n",
        parts.join("; ")
    )
}

/// Lower bound of a section's pick range: `"3-5"` → `3`.
pub fn parse_pick_min(pick_spec: Option<&str>) -> Option<u32> {
    let s = pick_spec?.trim_start();
    let digits: String = s.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// Hard-collapse same-event rewrites among the selected bullets.
///
/// Call after marker validation and before precheck, while `#N` identity still
/// exists, and only over the selected subset rather than the whole feed.
///
/// Greedy, deliberately not connected components: walk the model's order and
/// keep a bullet when its token overlap with every already-kept bullet is
/// below the threshold. Direct duplicates collapse, but a weak bridge cannot
/// transitively delete two unrelated stories — an A–B and a B–C edge must not
/// force dropping C when A and C barely overlap. Keeping the first in the
/// model's order also preserves its free-source preference where it already
/// expressed one.
pub fn post_dedup_selected_summary(
    summary: &str,
    numbered: &NumberedMap,
    section: &str,
    min_overlap: usize,
    pick_min: Option<u32>,
) -> String {
    let known: HashSet<u32> = numbered.keys().copied().collect();
    let selected = leading_marker_ids(summary, &known);

    if !llm_post_dedup_enabled() {
        log_trace(
            "llm_post_dedup",
            json!({"section": section, "enabled": false, "before": selected,
                   "after": selected, "dropped": [], "min_overlap": min_overlap}),
        );
        return summary.to_string();
    }

    if selected.len() < 2 {
        log_trace(
            "llm_post_dedup",
            json!({"section": section, "enabled": true, "before": selected,
                   "after": selected, "dropped": [], "pairs": [],
                   "min_overlap": min_overlap}),
        );
        return summary.to_string();
    }

    let word_sets: BTreeMap<u32, HashSet<String>> = selected
        .iter()
        .filter(|i| numbered.contains_key(i))
        .map(|i| (*i, topic_words(&numbered[i].title)))
        .collect();

    // Reported for diagnosis only; the collapse below is greedy, not union-find.
    let subset: NumberedMap = selected
        .iter()
        .filter_map(|i| numbered.get(i).map(|n| (*i, n.clone())))
        .collect();
    let pairs = dedup_pair_hints(&subset, min_overlap);

    let empty = HashSet::new();
    let mut keep_ordered: Vec<u32> = Vec::new();
    for mid in &selected {
        let words = word_sets.get(mid).unwrap_or(&empty);
        let collides = keep_ordered.iter().any(|k| {
            words
                .intersection(word_sets.get(k).unwrap_or(&empty))
                .count()
                >= min_overlap
        });
        if !collides {
            keep_ordered.push(*mid);
        }
    }

    let keep: HashSet<u32> = keep_ordered.iter().copied().collect();
    let after = keep_ordered.clone();
    let dropped: Vec<u32> = selected.iter().copied().filter(|m| !keep.contains(m)).collect();

    log_trace(
        "llm_post_dedup",
        json!({
            "section": section, "enabled": true, "before": selected,
            "after": after, "dropped": dropped,
            "pairs": pairs.iter()
                .map(|(a, b, ov)| json!({"a": a, "b": b, "overlap": ov}))
                .collect::<Vec<_>>(),
            "min_overlap": min_overlap
        }),
    );

    let mut body = summary.to_string();
    if !dropped.is_empty() {
        let bullet_lines: HashSet<&str> = news_bullet_lines(summary).into_iter().collect();
        let out: Vec<&str> = summary
            .lines()
            .filter(|line| {
                if !bullet_lines.contains(line) {
                    return true;
                }
                match leading_marker(line) {
                    Some(num) if numbered.contains_key(&num) => keep.contains(&num),
                    _ => true,
                }
            })
            .collect();
        body = out.join("\n");
    }

    if let Some(pick_min) = pick_min {
        let n = after.len() as u32;
        if selected.len() as u32 >= pick_min && n > 0 && n < pick_min {
            log_trace(
                "post_dedup_underfill",
                json!({"section": section, "before": selected.len(), "after": after.len(),
                       "pick_min": pick_min, "before_ids": selected, "after_ids": after}),
            );
            // The refilled id list is not returned: the caller re-derives the
            // selection from the rendered body, so a second copy could drift.
            let (new_body, _) = refill_unselected_after_underfill(
                &body, numbered, section, &after, &selected, pick_min, min_overlap,
            );
            body = new_body;
        }
    }
    body
}

/// One deterministic top-up from candidates the model never selected.
///
/// Does not re-call the model, does not lower the overlap threshold, and does
/// not revive anything the model did select — including bullets the collapse
/// above just dropped, which are duplicates by construction.
fn refill_unselected_after_underfill(
    summary: &str,
    numbered: &NumberedMap,
    section: &str,
    keep_ids: &[u32],
    llm_selected: &[u32],
    pick_min: u32,
    min_overlap: usize,
) -> (String, Vec<u32>) {
    let mut keep = keep_ids.to_vec();
    if pick_min == 0 || keep.len() as u32 >= pick_min || keep.is_empty() {
        log_trace(
            "post_dedup_refill",
            json!({"section": section, "attempted": 0, "added": [],
                   "final_count": keep.len(),
                   "still_underfill": (keep.len() as u32) < pick_min}),
        );
        return (summary.to_string(), keep);
    }

    let forbidden: HashSet<u32> = llm_selected.iter().copied().collect();
    let mut keep_word_sets: BTreeMap<u32, HashSet<String>> = keep
        .iter()
        .filter(|i| numbered.contains_key(i))
        .map(|i| (*i, topic_words(&numbered[i].title)))
        .collect();

    let mut added: Vec<u32> = Vec::new();
    let mut lines: Vec<String> = if summary.trim().is_empty() {
        Vec::new()
    } else {
        vec![summary.to_string()]
    };

    for (cand, item) in numbered {
        if keep.len() as u32 >= pick_min {
            break;
        }
        if forbidden.contains(cand) || keep_word_sets.contains_key(cand) {
            continue;
        }
        let title = item.title.trim();
        if title.is_empty() {
            continue;
        }
        let words = topic_words(title);
        if keep_word_sets
            .values()
            .any(|kw| words.intersection(kw).count() >= min_overlap)
        {
            continue;
        }
        keep.push(*cand);
        keep_word_sets.insert(*cand, words);
        added.push(*cand);
        lines.push(format!("- #{cand} {}", neutralize_markdown(title)));
    }

    log_trace(
        "post_dedup_refill",
        json!({"section": section, "attempted": added.len(), "added": added,
               "final_count": keep.len(),
               "still_underfill": (keep.len() as u32) < pick_min,
               "pick_min": pick_min}),
    );
    (lines.join("\n"), keep)
}

pub fn default_post_dedup(
    summary: &str,
    numbered: &NumberedMap,
    section: &str,
    pick_min: Option<u32>,
) -> String {
    post_dedup_selected_summary(summary, numbered, section, LLM_POST_DEDUP_OVERLAP, pick_min)
}
