//! The pipeline from candidate headlines to a delivered digest.
//!
//! One LLM call per section, each followed by the same sequence: marker and
//! shape gates, hard dedup of the selection, body precheck, paywall
//! replacement, language gate, link attachment. The order matters — dedup and
//! precheck both key on `#N`, which stops existing once links are attached.

use crate::agent::{run_agent, AgentResult};
use crate::alert::{alert_failure, AlertContext};
use crate::cache;
use crate::config::{
    self, llm_dedup_hints_enabled, section, AI_SUBSTAGE_CACHE_VARIANT, AI_SUBSTAGE_TIMEOUT_SECS,
    DEDUP_RULES, LLM_CUSTOM_TOPIC_LIMIT, LLM_DEDUP_HINT_OVERLAP, LLM_SECTION_TIMEOUT_SECS,
    LLM_TRANSLATION_TIMEOUT_SECS, TRANSLATION_RULES_STRICT,
};
use crate::precheck::{
    precheck_apply, render_replacements, resolve_paywall_replacements,
    resolve_paywall_summaries, PaywallMap, SharedCache,
};
use crate::render::{
    attach_numbered_links, markdown_visible_text, strip_links_keep_spacing, LinkMap, Numbered,
    PAYWALL_NOTE,
};
use crate::select::{
    default_pair_hints, dedup_pair_hints, format_dedup_hint_block, number_items_for_prompt,
    parse_pick_min, post_dedup_selected_summary, NumberedMap,
};
use crate::text::{title_without_source, Item};
use crate::trace::{clip_subprocess_text, log_trace, sample_nonempty_lines};
use crate::validate::{
    count_cjk, language_ok, language_stats, leading_marker_ids, marker_stats, neutralize_markdown,
    news_bullet_lines, shape_ok,
};
use serde_json::json;
use std::collections::HashSet;

pub const NO_NEWS: &str = "- 今日無相關新聞";

/// The AI section could not be produced even after subdividing.
///
/// Propagated so `main` exits non-zero without delivering a partial digest.
/// The alert is raised where the failure is detected, not here.
#[derive(Debug)]
pub struct AiSubstageExhausted(pub String);

fn counts_of(items: &[(String, Vec<Item>)]) -> Vec<(String, usize)> {
    items.iter().map(|(k, v)| (k.clone(), v.len())).collect()
}

fn known_ids(numbered: &NumberedMap) -> HashSet<u32> {
    numbered.keys().copied().collect()
}

/// The soft pair hints, plus the trace that records whether they were used.
fn hint_block_for(section_key: &str, numbered: &NumberedMap) -> String {
    if !llm_dedup_hints_enabled() {
        log_trace(
            "llm_dedup_hints",
            json!({"section": section_key, "enabled": false, "pairs": [],
                   "min_overlap": LLM_DEDUP_HINT_OVERLAP}),
        );
        return String::new();
    }
    let pairs = default_pair_hints(numbered);
    log_trace(
        "llm_dedup_hints",
        json!({"section": section_key, "enabled": true,
               "pairs": pairs.iter()
                   .map(|(a, b, ov)| json!({"a": a, "b": b, "overlap": ov}))
                   .collect::<Vec<_>>(),
               "min_overlap": LLM_DEDUP_HINT_OVERLAP}),
    );
    format_dedup_hint_block(&pairs)
}

/// Everything the diagnosis of a rejected reply needs, in one place.
struct Rejected<'a> {
    variant: &'a str,
    result: &'a AgentResult,
    summary: &'a str,
    marked: usize,
    total: usize,
    numbered: &'a NumberedMap,
    reason: &'a str,
    extra: Option<serde_json::Value>,
}

fn log_validation_failed(r: Rejected) {
    let Rejected {
        variant,
        result,
        summary,
        marked,
        total,
        numbered,
        reason,
        extra,
    } = r;
    let mut fields = json!({
        "variant": variant,
        "reason": reason,
        "returncode": result.returncode,
        "stdout_len": result.stdout.chars().count(),
        "stderr_len": result.stderr.chars().count(),
        "items_numbered": numbered.len(),
        "marked_bullets": marked,
        "total_bullets": total,
        "stdout_sample": clip_subprocess_text(summary, 1200),
        "line_sample": sample_nonempty_lines(summary, 8),
        "bullet_sample": news_bullet_lines(summary).iter().take(8)
            .map(|l| l.trim().chars().take(240).collect::<String>())
            .collect::<Vec<_>>(),
    });
    if let (Some(obj), Some(serde_json::Value::Object(e))) = (fields.as_object_mut(), extra) {
        for (k, v) in e {
            obj.insert(k, v);
        }
    }
    log_trace("llm_validation_failed", fields);
}

// ── translation ──────────────────────────────────────────────────────────────

/// Re-ask the model for Chinese headlines for an already-chosen set.
///
/// `None` means the retry did not produce something deliverable, and the
/// caller must treat the section as failed rather than ship half-English text.
pub fn translate_selected_section(
    key: &str,
    selected_ids: &[u32],
    numbered: &NumberedMap,
    date_str: &str,
    paywall: &PaywallMap,
) -> Option<Vec<String>> {
    if selected_ids.is_empty() {
        return None;
    }
    let selected: NumberedMap = selected_ids
        .iter()
        .filter_map(|i| numbered.get(i).map(|n| (*i, n.clone())))
        .collect();
    let raw = selected
        .iter()
        .map(|(idx, item)| format!("#{idx} {}", item.title))
        .collect::<Vec<_>>()
        .join("\n");

    let prompt = format!(
        "你是新聞標題翻譯編輯。以下是今天({date_str})已選出的新聞標題，每則有編號 #N。\n\n\
{raw}\n\n\
{translation_rules}\n\
輸出格式必須只有 dash bullets：\n\
- #N 繁體中文標題\n\
- #N ...\n\n\
每行必須以繁體中文新聞句子開始；不要先輸出英文原標題，也不要用「英文（中文）」格式。\n\
不要輸出開場白、區塊標題、解釋或英文原標題。",
        translation_rules = &*TRANSLATION_RULES_STRICT,
    );

    let counts = vec![(key.to_string(), selected.len())];
    let result = run_agent(
        &prompt,
        LLM_TRANSLATION_TIMEOUT_SECS,
        &format!("default_{key}_translate"),
        &counts,
        &selected,
    );
    let summary = result.stdout.trim().to_string();
    let known = known_ids(&selected);
    let (marked, total) = marker_stats(&summary, &known);
    let (chinese, lang_total) = language_stats(&summary);

    if total > 0 && marked == total && language_ok(&summary) {
        let (with_links, attached) =
            attach_numbered_links(&summary, &selected.iter().map(|(k, v)| (*k, v.clone())).collect(), &render_replacements(paywall));
        if attached > 0 {
            return Some(with_links.lines().map(str::to_string).collect());
        }
    }

    log_validation_failed(Rejected {
        variant: &format!("default_{key}_translate"),
        result: &result,
        summary: &summary,
        marked,
        total,
        numbered: &selected,
        reason: "translation_retry_validation",
        extra: Some(json!({"chinese_bullets": chinese, "language_total": lang_total})),
    });
    None
}

/// Translate exactly one headline, by reusing the section translator.
///
/// A non-empty placeholder link is required: the section translator only
/// reports success once at least one link attaches, so a blank link would make
/// every single-title translation look like a failure. The placeholder is
/// stripped back out below.
pub fn translate_single_title(title: &str, date_str: &str) -> Option<String> {
    let mut temp = NumberedMap::new();
    temp.insert(
        1,
        Numbered {
            title: title.to_string(),
            link: "https://paywall-rep.invalid/x".to_string(),
            source_name: String::new(),
        },
    );
    let lines = translate_selected_section("paywall_rep", &[1], &temp, date_str, &PaywallMap::new())?;
    let first = lines.first()?;
    let body = strip_links_keep_spacing(first);
    let body = body.trim_start();
    let body = body.strip_prefix('-').unwrap_or(body).trim();
    (!body.is_empty()).then(|| body.to_string())
}

/// Decide a cross-language replacement candidate, and render its headline.
///
/// One call answers both questions because they are the same question to a
/// reader: if this free Chinese article covers the paywalled English story,
/// what should the bullet say? Splitting it into a judgement call and a
/// translation call would double the model latency inside a 20-second pass for
/// no extra information.
///
/// `None` means "do not use this candidate" — for a genuinely different event,
/// for an unusable answer, and for an agent that failed. All three are the same
/// decision here, and the caller simply tries the next candidate.
pub fn judge_cross_language_candidate(
    orig_title: &str,
    cand_title: &str,
    date_str: &str,
) -> Option<String> {
    let orig = title_without_source(orig_title);
    let cand = title_without_source(cand_title);
    let prompt = format!(
        "你是新聞編輯。今天是 {date_str}。判斷以下兩則標題是否報導**同一則新聞事件**。\n\n\
原文標題（付費牆）：{orig}\n\
候選標題（免費）：{cand}\n\n\
判準：同一份研究／同一份報告／同一場發布會／同一起事件的不同語言、不同媒體改寫，\
算同一事件（標題角度不同不影響）。只是同一產業、同一家公司或同一個主題，但講的是不同事件，不算。\n\n\
只輸出一行，不要解釋、不要引號、不要編號：\n\
- 若是同一事件：輸出候選標題的繁體中文新聞標題（候選本身已是繁體中文就直接沿用）\n\
- 若不是同一事件：輸出 NO",
    );

    let result = run_agent(
        &prompt,
        LLM_TRANSLATION_TIMEOUT_SECS,
        "paywall_rep_cross_lang",
        &[("paywall_rep_cross_lang".to_string(), 1)],
        &NumberedMap::new(),
    );
    let verdict = cross_language_verdict(&result.stdout);
    log_trace(
        "paywall_cross_lang_judge",
        json!({"rc": result.returncode, "same_event": verdict.is_some()}),
    );
    verdict
}

/// Summarise a paywalled article the reader cannot open.
///
/// Only ever called for an item with no free replacement, so this is the
/// difference between a bare headline and knowing what the story said. The
/// summary is drawn strictly from the fetched body — the prompt forbids
/// outside knowledge, because a model filling gaps from memory would attribute
/// invented claims to a named publisher.
pub fn summarise_paywalled(title: &str, body: &str, date_str: &str) -> Option<String> {
    let clean = title_without_source(title);
    let prompt = format!(
        "你是新聞編輯。今天是 {date_str}。以下是一篇讀者無法開啟（付費牆）的文章全文。\n\n\
標題：{clean}\n\n\
內文：\n{body}\n\n\
請用繁體中文寫出這篇報導的重點摘要，2 到 3 句，每句一行，每行以 `- ` 開頭。\n\
規則：\n\
- 只根據上面的內文，不要加入內文沒有的資訊或你自己的背景知識\n\
- 寫這篇報導說了什麼，不要寫「這篇文章討論了…」這種轉述句\n\
- 不要輸出標題、開場白、結語或任何解釋\n\
- 若內文不足以判斷報導內容，只輸出 NO",
    );

    let result = run_agent(
        &prompt,
        LLM_TRANSLATION_TIMEOUT_SECS,
        "paywall_summary",
        &[("paywall_summary".to_string(), 1)],
        &NumberedMap::new(),
    );
    let out = paywall_summary_lines(&result.stdout);
    log_trace(
        "paywall_summary_agent",
        json!({"rc": result.returncode, "written": out.is_some()}),
    );
    out
}

/// Parse the summary: two or three Chinese lines, or `None`.
///
/// Kept separate from the agent call so the parse is testable without one. The
/// Chinese floor is applied per line rather than to the whole block: a model
/// that half-complies by echoing an English sentence between two Chinese ones
/// would pass a whole-block check and put English into a Chinese digest, past
/// the section language gate that has already run.
pub fn paywall_summary_lines(stdout: &str) -> Option<String> {
    let mut kept: Vec<String> = Vec::new();
    for line in stdout.lines() {
        let l = line.trim();
        if l.is_empty() {
            continue;
        }
        let l = l.strip_prefix('-').unwrap_or(l).trim();
        let l = markdown_visible_text(l);
        let l = l.trim();
        if l.is_empty() || l.eq_ignore_ascii_case("no") {
            continue;
        }
        if count_cjk(l) < 2 {
            continue;
        }
        kept.push(l.to_string());
        if kept.len() == 3 {
            break;
        }
    }
    (!kept.is_empty()).then(|| kept.join("\n"))
}

/// Parse the judge's answer: a headline, or `None` for a refusal.
///
/// Kept separate from the agent call so the parse is testable without one. The
/// Chinese-character floor is the load-bearing check: a model that ignores the
/// instruction usually does so by echoing the English original back, and
/// rendering that as the free replacement would put an English headline into a
/// Chinese digest — the exact failure the section language gate exists to
/// prevent, arriving after that gate has already run.
pub fn cross_language_verdict(stdout: &str) -> Option<String> {
    let line = stdout.lines().map(str::trim).find(|l| !l.is_empty())?;
    let line = line.strip_prefix('-').unwrap_or(line).trim();
    let line = markdown_visible_text(line);
    let line = line.trim();
    if line.is_empty() || line.eq_ignore_ascii_case("no") {
        return None;
    }
    (count_cjk(line) >= 2).then(|| line.to_string())
}

// ── the non-LLM fallback ─────────────────────────────────────────────────────

/// Raw headlines with links, used when the model path fails.
///
/// An untranslated English AI headline is replaced by a placeholder: the AI
/// section is read as Chinese, and a raw English title there reads as a bug
/// rather than as news.
pub fn fallback_section_lines(
    key: &str,
    items: &[Item],
    limit: usize,
    link_map: &LinkMap,
) -> Vec<String> {
    if items.is_empty() {
        return vec![NO_NEWS.to_string()];
    }
    items
        .iter()
        .take(limit)
        .enumerate()
        .map(|(i, item)| {
            let idx = i + 1;
            let mut title = item.title.clone();
            let link = link_map
                .get(&item.title)
                .map(str::to_string)
                .unwrap_or_else(|| item.link.clone());
            if key == "ai" && count_cjk(&title) < 2 {
                title = format!("AI 新聞來源 {idx}（摘要翻譯暫時失敗）");
            }
            let title = neutralize_markdown(&title);
            if link.is_empty() {
                format!("- {title}")
            } else {
                format!("- {title} [🔗]({link})")
            }
        })
        .collect()
}

pub fn fallback_summary(
    all_items: &[(String, Vec<Item>)],
    date_str: &str,
    link_map: &LinkMap,
) -> String {
    let mut lines = vec![format!("📰 早安新聞摘要 — {date_str}\n")];
    for (key, header, limit) in [
        ("ai", "**🤖 AI 人工智慧**", 10usize),
        ("tech", "**💻 科技 & 半導體**", 8),
        ("general", "**🌏 重大新聞**", 3),
    ] {
        let items: &[Item] = all_items
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_slice())
            .unwrap_or(&[]);
        lines.push(header.to_string());
        if items.is_empty() {
            lines.push(NO_NEWS.to_string());
        } else {
            for item in items.iter().take(limit) {
                let title = neutralize_markdown(&item.title);
                let link = link_map
                    .get(&item.title)
                    .map(str::to_string)
                    .unwrap_or_else(|| item.link.clone());
                if link.is_empty() {
                    lines.push(format!("- {title}"));
                } else {
                    lines.push(format!("- {title} [🔗]({link})"));
                }
            }
        }
        lines.push(String::new());
    }
    lines.join("\n")
}

// ── one default section ──────────────────────────────────────────────────────

/// `(lines, used_fallback)`.
///
/// `used_fallback` is true only when the LLM path failed and the raw listing
/// stood in for it. An empty input returns the placeholder with `false`,
/// because no model call was attempted and there is nothing to alert about.
pub fn summarize_default_section(
    key: &str,
    items: &[Item],
    date_str: &str,
    link_map: &LinkMap,
    cache_handle: &SharedCache,
) -> (Vec<String>, bool) {
    let spec = section(key).expect("known section");
    if items.is_empty() {
        return (vec![NO_NEWS.to_string()], false);
    }

    let section_items = vec![(key.to_string(), items.to_vec())];
    let (numbered, raw) = number_items_for_prompt(
        &section_items,
        Some(&[key.to_string()]),
        &|_| Some(spec.limit),
    );
    let hint_block = hint_block_for(key, &numbered);

    let prompt = format!(
        "你是新聞編輯。以下是今天({date_str})的「{header}」候選新聞標題（每則有編號 #N）。\n\n\
{raw}\n\n{hint_block}請挑出 {pick} 則{focus}。\n\
用繁體中文輸出，格式嚴格如下（不要輸出標題、開場白或結語）：\n\
- #N 新聞標題\n\
- #N ...\n\n\
規則：\n\
- 每則新聞前面必須保留原始編號 #N\n\
- {translation_rules}\n\
- 每行必須以繁體中文新聞句子開始，不要輸出英文原標題或「英文（中文）」格式\n\
- 排除瑣碎的、純行銷推廣的、政治宣傳性質的、投資建議類新聞\n\
{DEDUP_RULES}",
        header = spec.header,
        pick = spec.pick,
        translation_rules = &*TRANSLATION_RULES_STRICT,
        focus = spec.focus,
    );

    let counts = counts_of(&section_items);
    let result = run_agent(
        &prompt,
        LLM_SECTION_TIMEOUT_SECS,
        &format!("default_{key}"),
        &counts,
        &numbered,
    );
    let summary = result.stdout.trim().to_string();
    let known = known_ids(&numbered);

    if summary.is_empty() {
        log_validation_failed(Rejected {
            variant: &format!("default_{key}"),
            result: &result,
            summary: &summary,
            marked: 0,
            total: 0,
            numbered: &numbered,
            reason: "empty_stdout",
            extra: None,
        });
        eprintln!("[WARN] LLM section validation failed: section={key} empty stdout");
        return (
            fallback_section_lines(key, items, spec.fallback_limit, link_map),
            true,
        );
    }

    let (marked, total) = marker_stats(&summary, &known);
    if total == 0 || marked != total || !shape_ok(&summary, &known) {
        let reason = if total == 0 || marked != total {
            "marker_validation"
        } else {
            "shape_validation"
        };
        log_validation_failed(Rejected {
            variant: &format!("default_{key}"),
            result: &result,
            summary: &summary,
            marked,
            total,
            numbered: &numbered,
            reason,
            extra: None,
        });
        eprintln!("[WARN] LLM section validation failed: section={key} marked={marked}/{total}");
        return (
            fallback_section_lines(key, items, spec.fallback_limit, link_map),
            true,
        );
    }

    // Collapse same-event rewrites while `#N` identity still exists, before
    // precheck builds its paywall map from those same ids.
    let summary = post_dedup_selected_summary(
        &summary,
        &numbered,
        key,
        config::LLM_POST_DEDUP_OVERLAP,
        parse_pick_min(Some(spec.pick)),
    );
    if news_bullet_lines(&summary).is_empty() {
        log_trace(
            "quality_all_dropped",
            json!({"section": key, "reason": "post_dedup_empty"}),
        );
        return (vec![NO_NEWS.to_string()], false);
    }

    let language_passed = language_ok(&summary);
    if !language_passed {
        let (chinese, lang_total) = language_stats(&summary);
        log_validation_failed(Rejected {
            variant: &format!("default_{key}"),
            result: &result,
            summary: &summary,
            marked,
            total,
            numbered: &numbered,
            reason: "language_validation",
            extra: Some(json!({"chinese_bullets": chinese, "language_total": lang_total})),
        });
    }

    let (summary, mut paywall) = precheck_apply(&summary, &numbered, key, cache_handle);
    if news_bullet_lines(&summary).is_empty() {
        // Every pick was denied or promotional. That is the filter working, not
        // the model failing, so `used_fallback` stays false and no alert fires.
        log_trace("quality_all_dropped", json!({"section": key}));
        return (vec![NO_NEWS.to_string()], false);
    }
    resolve_paywall_replacements(
        &mut paywall,
        date_str,
        &translate_single_title,
        &judge_cross_language_candidate,
    );
    resolve_paywall_summaries(&mut paywall, date_str, &summarise_paywalled);

    if !language_passed {
        let selected = leading_marker_ids(&summary, &known);
        if let Some(translated) =
            translate_selected_section(key, &selected, &numbered, date_str, &paywall)
        {
            return (translated, false);
        }
    } else {
        let (with_links, attached) = attach_numbered_links(
            &summary,
            &numbered.iter().map(|(k, v)| (*k, v.clone())).collect(),
            &render_replacements(&paywall),
        );
        if attached > 0 {
            return (with_links.lines().map(str::to_string).collect(), false);
        }
        log_trace(
            "llm_link_validation_failed",
            json!({"variant": format!("default_{key}"), "section": key,
                   "returncode": result.returncode,
                   "stdout_len": result.stdout.chars().count(),
                   "items_numbered": numbered.len(),
                   "marked_bullets": marked, "total_bullets": total,
                   "stdout_sample": clip_subprocess_text(&summary, 1200),
                   "line_sample": sample_nonempty_lines(&summary, 8)}),
        );
    }

    eprintln!("[WARN] LLM section validation failed: section={key} marked={marked}/{total}");
    (
        fallback_section_lines(key, items, spec.fallback_limit, link_map),
        true,
    )
}

// ── the AI section, in batches ───────────────────────────────────────────────

/// `Ok(lines)` — validated bullets for `items[start..end]`, possibly empty when
/// every pick was filtered out. `Err(reason)` — a hard failure worth escalating.
///
/// A success is cached per (date, variant, range) and returned from cache on the
/// next attempt of the same range. An *empty* success is deliberately not
/// cached: a transient mis-drop must not stick for the rest of the day.
pub fn run_ai_substage(
    items: &[Item],
    start: usize,
    end: usize,
    date_str: &str,
    cache_handle: &SharedCache,
) -> Result<Vec<String>, String> {
    let variant = AI_SUBSTAGE_CACHE_VARIANT;
    if let Some(cached) = cache::get(date_str, variant, start, end) {
        return Ok(cached.lines().map(str::to_string).collect());
    }

    let sub_items = &items[start.min(items.len())..end.min(items.len())];
    if sub_items.is_empty() {
        return Ok(Vec::new());
    }

    let section_items = vec![("ai".to_string(), sub_items.to_vec())];
    let n_sub = sub_items.len();
    let (numbered, raw) =
        number_items_for_prompt(&section_items, Some(&["ai".to_string()]), &|_| Some(n_sub));
    let spec = section("ai").expect("ai section");
    // Proportional to the batch, since the halves are concatenated and neither
    // should over-select on its own.
    let pick_count = (n_sub / 3).max(2);
    let hint_block = hint_block_for("ai", &numbered);

    let prompt = format!(
        "你是新聞編輯。以下是今天({date_str})的「{header}」候選新聞標題（每則有編號 #N），這是分批處理的批次。\n\n\
{raw}\n\n{hint_block}請從這個批次挑出 {pick_count} 則{focus}。\n\
用繁體中文輸出，格式嚴格如下（不要輸出標題、開場白或結語）：\n\
- #N 新聞標題\n\
- #N ...\n\n\
規則：\n\
- 每則新聞前面必須保留原始編號 #N\n\
- {translation_rules}\n\
- 排除瑣碎的、純行銷推廣的、政治宣傳性質的、投資建議類新聞\n\
{DEDUP_RULES}",
        header = spec.header,
        translation_rules = &*TRANSLATION_RULES_STRICT,
        focus = spec.focus,
    );

    let counts = counts_of(&section_items);
    let result = run_agent(
        &prompt,
        AI_SUBSTAGE_TIMEOUT_SECS,
        &format!("{variant}_{start}_{end}"),
        &counts,
        &numbered,
    );
    let summary = result.stdout.trim().to_string();

    if result.timed_out() {
        return Err(format!("timeout after {AI_SUBSTAGE_TIMEOUT_SECS}s"));
    }
    if result.returncode != 0 {
        return Err(format!("exit_code={}", result.returncode));
    }
    if summary.is_empty() {
        return Err("empty_stdout".to_string());
    }

    let known = known_ids(&numbered);
    let (marked, total) = marker_stats(&summary, &known);
    if total == 0 || marked != total {
        return Err(format!("marker_validation marked={marked}/{total}"));
    }
    if !shape_ok(&summary, &known) {
        return Err("shape_validation".to_string());
    }

    let summary = post_dedup_selected_summary(
        &summary,
        &numbered,
        "ai",
        config::LLM_POST_DEDUP_OVERLAP,
        Some(pick_count as u32),
    );
    if news_bullet_lines(&summary).is_empty() {
        log_trace(
            "ai_substage_all_dropped",
            json!({"range": [start, end], "reason": "post_dedup_empty"}),
        );
        return Ok(Vec::new());
    }

    let (summary, mut paywall) = precheck_apply(&summary, &numbered, "ai", cache_handle);
    if news_bullet_lines(&summary).is_empty() {
        // A filter success, not a model failure — the driver must not escalate.
        log_trace("ai_substage_all_dropped", json!({"range": [start, end]}));
        return Ok(Vec::new());
    }
    resolve_paywall_replacements(
        &mut paywall,
        date_str,
        &translate_single_title,
        &judge_cross_language_candidate,
    );
    resolve_paywall_summaries(&mut paywall, date_str, &summarise_paywalled);

    if !language_ok(&summary) {
        let selected = leading_marker_ids(&summary, &known);
        let Some(translated) =
            translate_selected_section("ai", &selected, &numbered, date_str, &paywall)
        else {
            return Err("language_validation".to_string());
        };
        cache::put(date_str, variant, start, end, &translated.join("\n"));
        return Ok(translated);
    }

    let (with_links, attached) = attach_numbered_links(
        &summary,
        &numbered.iter().map(|(k, v)| (*k, v.clone())).collect(),
        &render_replacements(&paywall),
    );
    if attached == 0 {
        return Err("no_links_attached".to_string());
    }
    cache::put(date_str, variant, start, end, &with_links);
    Ok(with_links.lines().map(str::to_string).collect())
}

/// Two halves, then one more cut on any half that failed.
///
/// A quarter that still fails is terminal: the alert goes out here and the
/// error propagates so no partial digest is delivered.
pub fn summarize_default_ai_substaged(
    items: &[Item],
    date_str: &str,
    ctx: &AlertContext,
    cache_handle: &SharedCache,
) -> Result<Vec<String>, AiSubstageExhausted> {
    if items.is_empty() {
        return Ok(vec![NO_NEWS.to_string()]);
    }

    let before_cluster = items.len();
    let clusters = crate::text::cluster(items);
    let items = crate::text::pick_representatives(&clusters, 1);
    log_trace(
        "cluster_dedup",
        json!({"before": before_cluster, "after": items.len(),
               "clusters_total": clusters.len(), "clusters_kept": items.len()}),
    );
    if items.is_empty() {
        return Ok(vec![NO_NEWS.to_string()]);
    }

    let n = items.len();
    let mid = n / 2;
    log_trace(
        "ai_substage_start",
        json!({"total_items": n, "level2_a": [0, mid], "level2_b": [mid, n]}),
    );

    let halves = [(0usize, mid), (mid, n)];
    let mut half_results: [Option<Vec<String>>; 2] = [None, None];
    let mut half_errors: [String; 2] = [String::new(), String::new()];

    for (i, (s, e)) in halves.iter().enumerate() {
        match run_ai_substage(&items, *s, *e, date_str, cache_handle) {
            Ok(lines) => half_results[i] = Some(lines),
            Err(err) => {
                log_trace(
                    "ai_substage_level2_failed",
                    json!({"half": i, "range": [s, e], "error": err}),
                );
                half_errors[i] = err;
            }
        }
    }

    for (i, (s, e)) in halves.iter().enumerate() {
        if half_results[i].is_some() {
            continue;
        }
        let sub_n = e - s;
        if sub_n <= 1 {
            let detail = format!(
                "default_ai Level 2 half items[{s}..{e}] failed with size {sub_n}, \
cannot subdivide further. Level 2 error: {}",
                half_errors[i]
            );
            alert_failure(ctx, "ai_substage_level3_failed", &detail);
            return Err(AiSubstageExhausted(detail));
        }

        let sub_mid = s + sub_n / 2;
        let quarters = [(*s, sub_mid), (sub_mid, *e)];
        log_trace(
            "ai_substage_level3_start",
            json!({"failed_half": i, "quarters": quarters}),
        );

        let mut merged: Vec<String> = Vec::new();
        for (qs, qe) in quarters {
            match run_ai_substage(&items, qs, qe, date_str, cache_handle) {
                Ok(lines) => merged.extend(lines),
                Err(err) => {
                    let cached_ok: Vec<[usize; 2]> = quarters
                        .iter()
                        .filter(|(a, b)| {
                            cache::get(date_str, AI_SUBSTAGE_CACHE_VARIANT, *a, *b).is_some()
                        })
                        .map(|(a, b)| [*a, *b])
                        .collect();
                    let detail = format!(
                        "default_ai Level 3 quarter items[{qs}..{qe}] failed: {err}; \
Level 2 half [{s}..{e}] error: {}; quarters cached so far: {cached_ok:?}",
                        half_errors[i]
                    );
                    alert_failure(ctx, "ai_substage_level3_failed", &detail);
                    return Err(AiSubstageExhausted(detail));
                }
            }
        }
        half_results[i] = Some(merged);
    }

    let mut final_lines: Vec<String> = Vec::new();
    for lines in half_results.into_iter().flatten() {
        final_lines.extend(lines);
    }
    if final_lines.is_empty() {
        log_trace("ai_substage_empty_after_merge", json!({"total_items": n}));
        return Ok(vec![NO_NEWS.to_string()]);
    }

    let counts = vec![("ai".to_string(), items.len())];
    let final_lines = crate::crossdedup::cross_dedup_ai(final_lines, date_str, &counts);

    log_trace(
        "ai_substage_complete",
        json!({"total_items": n, "total_bullets": final_lines.len()}),
    );
    Ok(final_lines)
}

// ── assembly ─────────────────────────────────────────────────────────────────

/// Title, then each section's header and content, then the paywall footer.
///
/// Shared by the normal path and the theming length-guard revert, so the two
/// cannot drift apart.
pub fn assemble_digest(
    date_str: &str,
    section_keys: &[&str],
    section_results: &[(String, Vec<String>)],
) -> (String, usize) {
    let mut lines = vec![format!("📰 早安新聞摘要 — {date_str}\n")];
    for key in section_keys {
        let spec = section(key).expect("known section");
        lines.push(spec.header.to_string());
        if let Some((_, content)) = section_results.iter().find(|(k, _)| k == key) {
            lines.extend(content.clone());
        }
        lines.push(String::new());
    }
    let digest = lines.join("\n");
    let paywall_count = digest.matches(PAYWALL_NOTE).count();
    if paywall_count > 0 {
        return (
            format!("{digest}\nℹ️ 本次含 {paywall_count} 則付費牆新聞（原文需訂閱）"),
            paywall_count,
        );
    }
    (digest, paywall_count)
}

// ── custom topics ────────────────────────────────────────────────────────────

fn custom_cache_path(date_str: &str, variant: &str, topic: &str) -> std::path::PathBuf {
    let safe_date = date_str
        .split_whitespace()
        .next()
        .unwrap_or("")
        .replace('/', "-");
    let safe_topic: String = topic
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .take(40)
        .collect();
    config::cache_dir()
        .join(safe_date)
        .join(format!("custom-{variant}-{safe_topic}.txt"))
}

/// One LLM call for one topic.
///
/// Per-topic granularity is the resumability unit: each call covers at most
/// `LLM_CUSTOM_TOPIC_LIMIT` items and finishes well inside a typical kill
/// window, and a success is cached so the next attempt of the same
/// (date, topic) costs nothing. The variant is part of the filename so a
/// precheck-logic bump actually invalidates same-day caches.
pub fn run_custom_topic(
    topic: &str,
    items: &[Item],
    date_str: &str,
    cache_handle: &SharedCache,
) -> Result<Vec<String>, String> {
    let variant = "custom_topic_v3_dedup";
    let path = custom_cache_path(date_str, variant, topic);
    match std::fs::read_to_string(&path) {
        Ok(cached) => {
            log_trace(
                "news_cache_hit",
                json!({"variant": variant, "topic": topic}),
            );
            return Ok(cached.lines().map(str::to_string).collect());
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => log_trace(
            "news_cache_read_error",
            json!({"variant": variant, "topic": topic, "error": e.to_string()}),
        ),
    }

    if items.is_empty() {
        return Ok(vec![NO_NEWS.to_string()]);
    }

    let capped: Vec<Item> = items.iter().take(LLM_CUSTOM_TOPIC_LIMIT).cloned().collect();
    let section_items = vec![(topic.to_string(), capped)];
    let (numbered, raw) = number_items_for_prompt(
        &section_items,
        Some(&[topic.to_string()]),
        &|_| Some(LLM_CUSTOM_TOPIC_LIMIT),
    );
    let section_key = format!("custom:{topic}");
    let hint_block = hint_block_for(&section_key, &numbered);

    let prompt = format!(
        "你是新聞編輯。以下是今天({date_str})關於「{topic}」的候選新聞標題（每則有編號 #N）。\n\n\
{raw}\n\n{hint_block}請從中挑出 2-4 則真正有影響力、有意義的新聞，排除瑣碎、純行銷推廣、政治宣傳性質的新聞。\n\
用繁體中文輸出，格式嚴格如下（不要輸出標題、開場白或結語）：\n\
- #N 新聞標題\n\
- #N ...\n\n\
規則：\n\
- 每則新聞前面必須保留原始編號 #N\n\
- {translation_rules}\n\
{DEDUP_RULES}\n\
- 如果今日無相關新聞，輸出「{NO_NEWS}」",
        translation_rules = &*TRANSLATION_RULES_STRICT,
    );

    let safe_topic: String = topic
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .take(40)
        .collect();
    let counts = counts_of(&section_items);
    let result = run_agent(
        &prompt,
        AI_SUBSTAGE_TIMEOUT_SECS,
        &format!("{variant}_{safe_topic}"),
        &counts,
        &numbered,
    );
    let summary = result.stdout.trim().to_string();

    if result.timed_out() {
        return Err(format!("timeout after {AI_SUBSTAGE_TIMEOUT_SECS}s"));
    }
    if result.returncode != 0 {
        return Err(format!("exit_code={}", result.returncode));
    }
    if summary.is_empty() {
        return Err("empty_stdout".to_string());
    }

    let known = known_ids(&numbered);
    let (marked, total) = marker_stats(&summary, &known);
    if total == 0 || marked != total {
        return Err(format!("marker_validation marked={marked}/{total}"));
    }
    if !shape_ok(&summary, &known) {
        return Err("shape_validation".to_string());
    }

    let summary = post_dedup_selected_summary(
        &summary,
        &numbered,
        &section_key,
        config::LLM_POST_DEDUP_OVERLAP,
        Some(2), // the prompt asks for 2-4
    );
    if news_bullet_lines(&summary).is_empty() {
        log_trace(
            "quality_all_dropped",
            json!({"section": section_key, "reason": "post_dedup_empty"}),
        );
        return Ok(vec![NO_NEWS.to_string()]);
    }

    let (summary, mut paywall) = precheck_apply(&summary, &numbered, &section_key, cache_handle);
    if news_bullet_lines(&summary).is_empty() {
        // Returns before the cache write below, so an empty result is never
        // persisted and cannot suppress the topic for the rest of the day.
        log_trace("quality_all_dropped", json!({"section": section_key}));
        return Ok(vec![NO_NEWS.to_string()]);
    }
    resolve_paywall_replacements(
        &mut paywall,
        date_str,
        &translate_single_title,
        &judge_cross_language_candidate,
    );
    resolve_paywall_summaries(&mut paywall, date_str, &summarise_paywalled);

    let write_cache = |body: &str| {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match std::fs::write(&path, body) {
            Ok(()) => log_trace(
                "news_cache_write",
                json!({"variant": variant, "topic": topic, "bytes": body.len()}),
            ),
            Err(e) => log_trace(
                "news_cache_write_error",
                json!({"variant": variant, "topic": topic, "error": e.to_string()}),
            ),
        }
    };

    // Same ordering as the AI substage, so a refilled raw English title still
    // gets translated rather than shipped as-is.
    if !language_ok(&summary) {
        let selected = leading_marker_ids(&summary, &known);
        let Some(translated) =
            translate_selected_section(&section_key, &selected, &numbered, date_str, &paywall)
        else {
            return Err("language_validation".to_string());
        };
        write_cache(&translated.join("\n"));
        return Ok(translated);
    }

    let (with_links, attached) = attach_numbered_links(
        &summary,
        &numbered.iter().map(|(k, v)| (*k, v.clone())).collect(),
        &render_replacements(&paywall),
    );
    if attached == 0 {
        // Readable bullets with no link is still useful, so this counts as a
        // success — but it is not cached, so a re-run can do better.
        return Ok(summary.lines().map(str::to_string).collect());
    }
    write_cache(&with_links);
    Ok(with_links.lines().map(str::to_string).collect())
}

/// The non-LLM listing for one topic.
pub fn custom_topic_raw_listing(items: &[Item], link_map: &LinkMap) -> Vec<String> {
    if items.is_empty() {
        return vec![NO_NEWS.to_string()];
    }
    items
        .iter()
        .take(5)
        .map(|item| {
            let title = neutralize_markdown(&item.title);
            let link = link_map
                .get(&item.title)
                .map(str::to_string)
                .unwrap_or_else(|| item.link.clone());
            if link.is_empty() {
                format!("- {title}")
            } else {
                format!("- {title} [🔗]({link})")
            }
        })
        .collect()
}

/// Unused directly, but kept as the single place the hint threshold for a
/// non-default section is chosen.
pub fn custom_pair_hints(numbered: &NumberedMap) -> Vec<(u32, u32, usize)> {
    dedup_pair_hints(numbered, LLM_DEDUP_HINT_OVERLAP)
}
