//! Driving the sections and assembling the message.

use crate::alert::{alert_failure, AlertContext};
use crate::config::{section, THEME_TRIM_THRESHOLD};
use crate::precheck::SharedCache;
use crate::render::{
    build_link_map, markdown_visible_text, trim_digest_links, trim_links_to_limit, PAYWALL_NOTE,
};
use crate::summarize::{
    assemble_digest, custom_topic_raw_listing, fallback_section_lines, run_custom_topic,
    summarize_default_ai_substaged, summarize_default_section, AiSubstageExhausted, NO_NEWS,
};
use crate::text::Item;
use crate::theme::theme_ai_section;
use crate::trace::log_trace;
use crate::validate::dropped_protected_names;
use serde_json::json;
use std::time::Instant;

pub const SECTION_KEYS: [&str; 3] = ["ai", "tech", "general"];

fn items_for<'a>(all_items: &'a [(String, Vec<Item>)], key: &str) -> &'a [Item] {
    all_items
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_slice())
        .unwrap_or(&[])
}

/// Record any company name that was in a source headline and is not in the
/// line the reader gets — translated away, or summarised away.
///
/// Called once the digest is final, because theming and the length revert can
/// both rewrite which lines ship. Observation only — see
/// [`crate::validate::dropped_protected_names`] for why this must not gate.
fn log_protected_name_losses(all_items: &[(String, Vec<Item>)], digest: &str) {
    let sources: Vec<(String, String)> = all_items
        .iter()
        .flat_map(|(_, items)| items)
        .map(|i| (i.title.clone(), i.link.clone()))
        .collect();
    let lines: Vec<String> = digest.lines().map(str::to_string).collect();
    let lost = dropped_protected_names(&sources, &lines);
    if lost.is_empty() {
        return;
    }
    log_trace(
        "protected_name_lost",
        json!({"count": lost.len(),
               "names": lost.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>(),
               "links": lost.iter().map(|(_, l)| l.clone()).collect::<Vec<_>>()}),
    );
    eprintln!(
        "[WARN] company name in the source headline did not reach the reader: {}",
        lost.iter()
            .map(|(n, _)| n.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
}

/// The default AI/tech/general digest.
///
/// There is no catch-all around each section the way the Python has one: every
/// failure mode here is in a return type, so the only thing a `catch` could
/// still catch is a panic, and a panic in this skill is a bug to fix rather
/// than a section to degrade.
pub fn summarize_llm(
    all_items: &[(String, Vec<Item>)],
    ctx: &AlertContext,
    date_str: &str,
    cache_handle: &SharedCache,
) -> Result<String, AiSubstageExhausted> {
    let link_map = build_link_map(all_items);
    let mut section_results: Vec<(String, Vec<String>)> = Vec::new();
    let mut degraded: Vec<String> = Vec::new();

    for key in SECTION_KEYS {
        let started = Instant::now();
        let result = if key == "ai" {
            summarize_default_ai_substaged(items_for(all_items, key), date_str, ctx, cache_handle)
        } else {
            let (lines, used_fallback) = summarize_default_section(
                key,
                items_for(all_items, key),
                date_str,
                &link_map,
                cache_handle,
            );
            if used_fallback && !items_for(all_items, key).is_empty() {
                degraded.push(key.to_string());
            }
            Ok(lines)
        };
        // Section wall clock covers the model call plus precheck, paywall
        // lookup, translation and cross-dedup — not just the agent's own
        // elapsed. Recorded even when the AI section is about to abort.
        log_trace(
            "section_timing",
            json!({"section": key, "elapsed_ms": started.elapsed().as_millis() as u64}),
        );
        section_results.push((key.to_string(), result?));
    }

    if !degraded.is_empty() {
        // News still goes out, but the reader asked for curated news and is
        // getting a raw bullet dump, so this counts as "could not send the
        // news" for alerting purposes.
        alert_failure(
            ctx,
            "section_fallback_used",
            &format!("sections delivered using non-LLM fallback: {degraded:?}"),
        );
    }

    let flat_ai = section_results
        .iter()
        .find(|(k, _)| k == "ai")
        .map(|(_, v)| v.clone());
    let mut themed_applied = false;
    if let Some(flat) = flat_ai.as_ref() {
        if !degraded.iter().any(|d| d == "ai") {
            let counts: Vec<(String, usize)> = all_items
                .iter()
                .map(|(k, v)| (k.clone(), v.len()))
                .collect();
            let (lines, applied) = theme_ai_section(flat, date_str, &counts);
            themed_applied = applied;
            if let Some(slot) = section_results.iter_mut().find(|(k, _)| k == "ai") {
                slot.1 = lines;
            }
        }
    }

    let (mut digest, paywall_count) = assemble_digest(date_str, &SECTION_KEYS, &section_results);
    if paywall_count > 0 {
        // The note appears exactly once per paywalled *story* — both the
        // replacement-pair and the degraded single-bullet forms carry one — so
        // this counts stories, not rendered bullets. Not a failure, so it never
        // routes through the alert path.
        log_trace("paywall_notice", json!({"count": paywall_count}));
    }

    if themed_applied {
        let visible = markdown_visible_text(&digest).chars().count();
        if visible > THEME_TRIM_THRESHOLD {
            // The headings pushed the digest into the trim path, which could
            // drop a block or stale the footer. Theming is never worth a drop,
            // so rebuild flat — byte-identical to having it switched off.
            if let (Some(flat), Some(slot)) = (
                flat_ai,
                section_results.iter_mut().find(|(k, _)| k == "ai"),
            ) {
                slot.1 = flat;
            }
            let rebuilt = assemble_digest(date_str, &SECTION_KEYS, &section_results);
            digest = rebuilt.0;
            log_trace(
                "ai_theme",
                json!({"mode": "render", "length_revert": true, "visible_len": visible}),
            );
        }
    }

    let digest = trim_digest_links(&digest);
    log_protected_name_losses(all_items, &digest);
    Ok(digest)
}

/// `topic=reason`, capped so a broad failure stays inside one readable line.
///
/// The reason travels because "(LLM failed)" has been covering four different
/// diseases: the trace holds 81 `custom_topic_fell_back` events since 2026-05-06
/// — 39 `marker_validation`, 24 `timeout`, 13 `shape_validation`, 4
/// `language_validation` — and each wants a different next step. A timeout is a
/// budget problem and is already retried; a `shape_validation` is the model
/// returning its analysis instead of the `- #N 標題` lines (the gate is
/// deliberate: `tests/validate.rs::reasoning_prose_is_invisible_to_the_bullet_list_but_visible_to_the_shape_gate`);
/// `marker_validation marked=0/0` was the NO_NEWS sentinel case fixed 2026-09-01.
/// Without the reason the operator cannot tell a recurring prompt problem from a
/// flaky provider, and the cluster counter mixes them into one number.
fn degraded_detail(degraded: &[(String, String)]) -> String {
    const CAP: usize = 5;
    let mut parts: Vec<String> = degraded
        .iter()
        .take(CAP)
        .map(|(topic, reason)| format!("{topic}={reason}"))
        .collect();
    if degraded.len() > CAP {
        parts.push(format!("…+{} more", degraded.len() - CAP));
    }
    parts.join("; ")
}

/// The per-topic digest, one model call per topic.
///
/// A topic whose call fails is replaced by a raw listing — still useful — and
/// alerted. Other topics deliver normally. If every topic falls back, a second
/// alert fires and the digest still ships raw rather than leaving the reader
/// with nothing.
pub fn summarize_llm_custom(
    all_items: &[(String, Vec<Item>)],
    topics: &[String],
    ctx: &AlertContext,
    date_str: &str,
    cache_handle: &SharedCache,
) -> String {
    let link_map = build_link_map(all_items);
    log_trace(
        "custom_substage_start",
        json!({"topic_count": topics.len(), "topics": topics}),
    );

    let mut sections: Vec<(String, Vec<String>)> = Vec::new();
    // (topic, why the call was rejected) — the reason is what makes the alert
    // actionable, see `degraded_detail`.
    let mut degraded: Vec<(String, String)> = Vec::new();

    for topic in topics {
        let items = items_for(all_items, topic);
        match run_custom_topic(topic, items, date_str, cache_handle) {
            Ok(lines) => sections.push((topic.clone(), lines)),
            Err(err) => {
                log_trace(
                    "custom_topic_fell_back",
                    json!({"topic": topic, "error": err}),
                );
                sections.push((topic.clone(), custom_topic_raw_listing(items, &link_map)));
                degraded.push((topic.clone(), err));
            }
        }
    }

    if !degraded.is_empty() {
        let detail = degraded_detail(&degraded);
        if degraded.len() == topics.len() {
            alert_failure(
                ctx,
                "all_custom_topics_failed",
                &format!(
                    "every custom topic LLM call failed; full digest is raw-listing only. topics={detail}"
                ),
            );
        } else {
            alert_failure(
                ctx,
                "custom_topics_fell_back",
                &format!("these topics delivered as raw listings: {detail}"),
            );
        }
    }

    let mut lines = vec![format!("📰 每日新聞摘要 — {date_str}\n")];
    for topic in topics {
        lines.push(format!("**{topic}**"));
        match sections.iter().find(|(t, _)| t == topic) {
            Some((_, content)) => lines.extend(content.clone()),
            None => lines.push(NO_NEWS.to_string()),
        }
        lines.push(String::new());
    }

    let mut digest = lines.join("\n");
    let paywall_count = digest.matches(PAYWALL_NOTE).count();
    if paywall_count > 0 {
        digest.push_str(&format!(
            "\nℹ️ 本次含 {paywall_count} 則付費牆新聞（原文需訂閱）"
        ));
        log_trace("paywall_notice", json!({"count": paywall_count}));
    }

    log_trace(
        "custom_substage_complete",
        json!({"topic_count": topics.len(), "degraded_count": degraded.len()}),
    );
    let digest = trim_links_to_limit(&digest, 4000);
    log_protected_name_losses(all_items, &digest);
    digest
}

/// The whole-digest non-LLM fallback, used when no section could be curated.
pub fn raw_digest(all_items: &[(String, Vec<Item>)], date_str: &str) -> String {
    let link_map = build_link_map(all_items);
    let results: Vec<(String, Vec<String>)> = SECTION_KEYS
        .iter()
        .map(|key| {
            let spec = section(key).expect("known section");
            (
                key.to_string(),
                fallback_section_lines(key, items_for(all_items, key), spec.fallback_limit, &link_map),
            )
        })
        .collect();
    assemble_digest(date_str, &SECTION_KEYS, &results).0
}

#[cfg(test)]
mod tests {
    use super::degraded_detail;

    fn pairs(v: &[(&str, &str)]) -> Vec<(String, String)> {
        v.iter()
            .map(|(t, r)| (t.to_string(), r.to_string()))
            .collect()
    }

    #[test]
    fn the_reason_rides_with_the_topic() {
        // "(LLM failed)" covered timeout, shape, marker and language rejections
        // alike; the alert has to name which one happened, because the next
        // step differs per reason.
        assert_eq!(
            degraded_detail(&pairs(&[("富人", "shape_validation")])),
            "富人=shape_validation"
        );
        assert_eq!(
            degraded_detail(&pairs(&[
                ("節稅", "timeout after 60s"),
                ("富人", "marker_validation marked=3/7")
            ])),
            "節稅=timeout after 60s; 富人=marker_validation marked=3/7"
        );
    }

    #[test]
    fn a_broad_failure_caps_the_list_and_says_how_much_it_dropped() {
        let many = pairs(&[
            ("a", "timeout after 60s"),
            ("b", "timeout after 60s"),
            ("c", "timeout after 60s"),
            ("d", "timeout after 60s"),
            ("e", "timeout after 60s"),
            ("f", "timeout after 60s"),
            ("g", "timeout after 60s"),
        ]);
        let detail = degraded_detail(&many);
        assert!(detail.starts_with("a=timeout after 60s"), "{detail}");
        assert!(detail.ends_with("…+2 more"), "{detail}");
        // five entries kept plus the tail note, so five separators
        assert_eq!(detail.matches("; ").count(), 5, "{detail}");
    }
}
