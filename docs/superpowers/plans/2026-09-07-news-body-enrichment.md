# News Body-Enrichment (P1) + Selection Preference (P2) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stop delivering digest items whose headline announces an artifact without naming it (e.g. "Meta 發布開放權重模型" with no model name) by feeding already-fetched Tier-2 article bodies into a targeted title-revision call, and preferring entity-naming titles at selection.

**Architecture:** Tier-2 precheck already fetches article bodies for selected items and throws them away after classification (`precheck_action`). We keep the body excerpt on the `Verdict`, thread a `num → excerpt` map out of `precheck_apply`, and — only when a deterministic trigger finds items whose excerpt names a Latin entity missing from the delivered zh title — run ONE extra nested-agent call per affected section to revise just those titles, with a deterministic output validator (every Latin token in a revised line must come from that item's title ∪ excerpt). P2 is one preference sentence in the three selection prompts. Custom-topic sections and paywall-replacement titles are out of v1 scope.

**Tech Stack:** Rust crate `news` in this workspace; nested agent via existing `nullclaw agent --isolated` runner (`crates/news/src/agent.rs`); HTTP only via `claw_core::http::agent` (no new network calls — excerpts reuse the existing `fetch_article_text` fetch).

**Spec:** This document. Owner approved the direction 2026-09-07 ("方向核准，先跑 Codex/Grok 驗證"). Codex verified the root-cause diagnosis (D1–D4) against source; adversarial review was to be Grok but Grok Build died twice with `402 Payment Required` (balance exhausted), so `kimi:kimi-challenge` ran the identical brief and returned **ship-with-changes**; its six changes are folded in here (mapping in "Design decisions"). Root-cause evidence: selection/translation see RSS titles only (`feed.rs:98-173`, prompts at `summarize.rs` ~455-480 / ~643-657 / ~949-961); translation call runs only on the `!language_ok` retry path (`summarize.rs:573-579`, `717-726`); no gate measures specificity.

## Design decisions (and where they came from)

1. **Targeted revision call, NOT unconditional translation.** Kimi's finding 1: the translation call only runs on the language-failure path, so enriching it changes almost nothing. Its proposed fix (i) — translate unconditionally — would add one LLM call per *custom topic* too, and `digest.rs:219-221` shows custom topics are one call per topic (17 topics ⇒ call count roughly doubles). Decision: keep the happy path, add a revision call **per default section, only when the deterministic trigger fires**. Custom topics: P2 only.
2. **Excerpts keyed by post-dedup `#N`, scoped per call, carried inside the cached `Verdict`** (Kimi change 2) — never in a link-keyed side table (shared-link replacement candidates, `precheck.rs:361-364`), and the memo round-trips them so AI Level-3 re-subdivision sees consistent data.
3. **Deterministic output validator** (Kimi change 3): every Latin token in a revised line must appear in that item's title ∪ excerpt; violating lines revert to the original. The codebase already saw a prompt-only rule misread ("可以保留英文原文" → 「擁抱臉書執行長」, `config.rs:261-266`), so the prompt rule alone is not trusted.
4. **Excerpt usability gate** (Kimi change 4): `error.is_none()` + word floor + challenge/chrome filter + title-token overlap; per-item omission on failure. Cloudflare "Just a moment" pages currently classify as `Keep` with garbage text — they must never reach a prompt.
5. **Cache variant bumps in the same commit** (Kimi change 5): `AI_SUBSTAGE_CACHE_VARIANT` (`config.rs:39`) because P1+P2 change AI output; `custom_topic_v3_dedup` (`summarize.rs:918`) because P2 changes the custom prompt. Default tech/general sections and the translation path are uncached (verified: `cache::put` only at `summarize.rs:724/736` and the custom `write_cache`), so nothing else bumps.
6. **Paywall-replacement titles and the `!language_ok` retry path are out of v1 scope** (Kimi change 6, extended): replacements already render a fetched-replacement title; the retry path is the minority path and already has its own rule set.

## Global Constraints

- Unknown CLI flag must exit 2 (install probe) — no CLI surface changes in this plan.
- Every HTTP call goes through `claw_core::http::agent(timeout)`; `tools/lint-http.sh` must stay green. This plan adds **zero** new network calls.
- No Python; everything in `crates/news/`. Offline tests only (`cargo test --workspace` needs no network/interpreter/API key).
- Scheduler markers / `finish()` / delivery path untouched.
- Expected clippy: zero warnings. Test-design anti-patterns: read `docs/specs/2026-07-28-phase1-lessons.md` before writing tests.
- Cost envelope: at most **one** extra nested-agent call per default section (tech, general, and each AI substage) per run, and only when the trigger fires; run-time watched via the existing `news_run_timing` trace.

---

### Task 1: Excerpt gate in `quality.rs` — `Verdict.body_excerpt` + `usable_excerpt`

**Files:**
- Modify: `crates/news/src/quality.rs` (struct `Verdict` ~:452-457; new fns near `fetch_article_text` ~:418-441; `Verdict::new` callers ~:530-588)
- Modify: `crates/news/src/config.rs` (new env-backed knobs beside the precheck knobs ~:85-130)
- Test: in-file `#[cfg(test)]` module (follow existing style, e.g. the `google_news_article_url_pub` hook ~:160-166)

**Interfaces:**
- Produces: `Verdict { action, reason, decoded_url, body_excerpt: Option<String> }`; `pub fn usable_excerpt(title: &str, article: &Article) -> Option<String>`; `config::excerpt_min_words() -> u32` (env `NEWS_EXCERPT_MIN_WORDS`, default 40); `config::excerpt_max_chars() -> usize` (env `NEWS_EXCERPT_MAX_CHARS`, default 600). `Verdict::new` gains one parameter; all existing callers updated in this task.

- [ ] **Step 1: Write the failing tests**

```rust
// quality.rs tests — fixture texts inline, all pure fns, no network.
#[test]
fn usable_excerpt_takes_leading_window_of_clean_body() {
    let a = article_with(None, &format!("{} {}", real_para(), real_para())); // >40 words
    let ex = usable_excerpt("Meta unveils Muse Glimmer open-weight model", &a).unwrap();
    assert!(ex.chars().count() <= excerpt_max_chars());
    assert!(ex.contains("Muse"));
}

#[test]
fn usable_excerpt_rejects_error_short_and_challenge_bodies() {
    assert!(usable_excerpt("t", &article_with(Some("boom"), "irrelevant")).is_none());
    assert!(usable_excerpt("Meta unveils Muse Glimmer", &article_with(None, "too short")).is_none());
    let cf = article_with(None, "Just a moment... Enable JavaScript and cookies to continue \
        cf-chl widget noscript please wait while we verify your browser identity token");
    assert!(usable_excerpt("Meta unveils Muse Glimmer", &cf).is_none());
}

#[test]
fn usable_excerpt_rejects_body_sharing_no_significant_token_with_title() {
    let body = "The committee published its annual report on railway scheduling delays today \
        across several regional lines with detailed tables and annexes for readers";
    assert!(usable_excerpt("Meta unveils Muse Glimmer open-weight model", &article_with(None, body)).is_none());
}

#[test]
fn verdict_new_still_builds_with_excerpt_none() {
    let v = Verdict::new(Action::Keep, None, None, None);
    assert_eq!(v.body_excerpt, None);
}
```

Helper `article_with(err: Option<&str>, text: &str) -> Article` mirrors how existing tests build `Article` (check the struct's fields at `quality.rs:418-441` and fill host/word_count the same way existing code computes them).

- [ ] **Step 2: Run to verify failure** — `cargo test -p news usable_excerpt` → FAIL (function not defined).

- [ ] **Step 3: Implement**

```rust
fn looks_like_challenge(text_lower: &str) -> bool {
    ["just a moment", "enable javascript", "checking your browser",
     "attention required", "cf-chl", "verify you are a human"]
        .iter().any(|m| text_lower.contains(m))
}

fn shares_significant_token(title: &str, text: &str) -> bool {
    let text = text.to_lowercase();
    token_iter(title)                      // lowercase [a-z0-9]+ runs, >=4 chars, >=1 ascii alphabetic
        .any(|t| text.contains(&t))
}

pub fn usable_excerpt(title: &str, article: &Article) -> Option<String> {
    if article.error.is_some() { return None; }
    let text = article.text.trim();
    let words = text.split_whitespace().count() as u32;
    if words < excerpt_min_words() { return None; }
    let lower = text.to_lowercase();
    if looks_like_challenge(&lower) { return None; }
    if !shares_significant_token(title, text) { return None; }
    let max = excerpt_max_chars();
    let end = text.char_indices().map(|(i, _)| i)
        .take_while(|&i| i <= max).last().unwrap_or(0)
        + text[max.min(text.len())..].len() * 0; // see note below
    // char-boundary-safe window: walk char_indices to the largest boundary <= max
    let cut = text.char_indices().map(|(i, _)| i)
        .filter(|&i| i <= max).last().map(|i| i + text[i..].chars().next().map_or(1,|c|c.len_utf8()) * 0)
        .unwrap_or(text.len());
    Some(text[..cut.max(cut)] /* replace with the correct boundary computed above; do not panic on multibyte */.to_string())
}
```

> The two `cut` expressions above are sketches of the same boundary walk — write it ONCE, cleanly: iterate `char_indices`, keep the last index `i` with `i + ch.len_utf8() <= max`, slice `&text[..i + ch.len_utf8()]`. The test `usable_excerpt_takes_leading_window_of_clean_body` must pass with CJK fixtures too (add one: body containing 臺灣中文 text past the 600-char mark).

Also: extend `Verdict` with `pub body_excerpt: Option<String>`, thread it through `Verdict::new(action, reason, decoded_url, body_excerpt)`, and pass `None` at every existing construction site except the final `classify_quality` branch in `precheck_action` (`quality.rs:583-588`), which computes `usable_excerpt(title, &article)` — **only on the `Keep` arm** (`Drop`/`TitleOnly` bodies stay `None`; that is Task 2's wiring, but `precheck_action` itself is touched here to keep the struct change compiling — minimal: `let excerpt = if action == Action::Keep { usable_excerpt(title, &article) } else { None };`).

Config knobs follow the crate's existing env-knob idiom (copy the shape of `precheck_fetch_timeout()` at `config.rs:85-93`).

- [ ] **Step 4: Run to verify pass** — `cargo test -p news` → all PASS (old tests included: `Verdict` sites updated).

- [ ] **Step 5: Commit** — `git add -A && git commit -m "feat(news): usable_excerpt gate + body_excerpt on Verdict"`

---

### Task 2: Thread `num → excerpt` out of `precheck_apply`

**Files:**
- Modify: `crates/news/src/precheck.rs` (`precheck_apply` ~:122-232 — returns `(String, PaywallMap, BTreeMap<u32, String>)`)
- Modify: `crates/news/src/summarize.rs` (three call sites: ~:558, ~:703, ~:1106)

**Interfaces:**
- Consumes: `Verdict.body_excerpt` (Task 1).
- Produces: `precheck_apply(...) -> (String, PaywallMap, BTreeMap<u32, String>)` — the map holds `num → excerpt` for **`Keep` verdicts only**; custom-topic call site binds `let (summary, paywall, _bodies) = ...`.

- [ ] **Step 1: Failing test** — in `precheck.rs` tests, build a `BTreeMap<u32, Verdict>` fixture (two Keep-with-excerpt, one Keep-without, one TitleOnly, one Drop), feed the existing map-building block's logic. If that block is not a free fn, extract it (`fn collect_bodies(verdicts: &BTreeMap<u32, Verdict>) -> BTreeMap<u32, String>`) and test THAT:

```rust
#[test]
fn collect_bodies_keeps_only_keep_verdicts_with_excerpt() {
    // Keep+Some -> present; Keep+None -> absent; TitleOnly/Drop -> absent
}
```

- [ ] **Step 2: Run** `cargo test -p news collect_bodies` → FAIL (fn not defined).
- [ ] **Step 3: Implement** `collect_bodies` + change `precheck_apply` return type; update the three call sites in `summarize.rs` (custom site discards with `_bodies`; the other two keep the binding alive for Task 5 — prefix `_` for now to stay clippy-clean: `(summary, paywall, bodies)` then `let _ = &bodies;` is NOT acceptable; instead name it and add `#[allow(dead_code)]`? No — cleanest: Task 5 uses it within the same PR, so temporarily `let (_, _, bodies) = ...; debug_assert!(!bodies.is_empty() || true);` is also ugly. **Decision: do Tasks 2 and 5 in one commit window; between them, keep the tree clippy-clean by binding as `let (summary, paywall, _bodies)` and removing the underscore in Task 5.**)
- [ ] **Step 4: Run** `cargo test -p news && cargo clippy -p news --all-targets` → PASS, zero warnings.
- [ ] **Step 5: Commit** — `feat(news): precheck_apply returns per-item body excerpts`

---

### Task 3: Latin-token validator in `validate.rs`

**Files:**
- Modify: `crates/news/src/validate.rs` (new pub fns beside the language gate ~:250-282)
- Test: in-file `#[cfg(test)]`

**Interfaces:**
- Produces: `pub fn latin_tokens(s: &str) -> HashSet<String>` (lowercase `[a-z0-9]+` runs, length ≥ 2, at least one ASCII alphabetic char); `pub fn line_tokens_covered(line: &str, allowed_a: &str, allowed_b: &str) -> bool` (`latin_tokens(line) ⊆ latin_tokens(a) ∪ latin_tokens(b)`); `pub fn has_cjk_2_in_head(line: &str) -> bool` (≥2 CJK chars within the first 18 chars — mirrors the existing bullet rule).

- [ ] **Step 1: Failing tests**

```rust
#[test]
fn latin_tokens_lowercases_and_filters() {
    let t = latin_tokens("Meta 發布 Muse Spark 1.2 開放權重");
    assert!(t.contains("meta") && t.contains("muse") && t.contains("spark"));
    assert!(!t.contains("1") && !t.contains("发布"));
}

#[test]
fn uncovered_token_is_detected() {
    let title = "Meta 發布開放權重模型";
    let excerpt = "Meta launched Muse Glimmer, a 30-billion-parameter model";
    assert!(line_tokens_covered("- #3 Meta 發布 Muse Glimmer 開放權重模型", title, excerpt));
    assert!(!line_tokens_covered("- #3 Meta 發布 Llama 5 開放權重模型", title, excerpt)); // hallucinated
}

#[test]
fn cjk_head_check_mirrors_language_gate() {
    assert!(has_cjk_2_in_head("- #3 Meta 發布開放權重模型"));
    assert!(!has_cjk_2_in_head("- #3 Meta launches new AI model"));
}
```

- [ ] **Step 2: Run** → FAIL. - [ ] **Step 3: Implement** the three fns. - [ ] **Step 4: Run** → PASS. - [ ] **Step 5: Commit** — `feat(news): latin-token coverage validator for enriched titles`

---

### Task 4: Revision-call pure core in `summarize.rs` — trigger, prompt, sanitizer

**Files:**
- Modify: `crates/news/src/summarize.rs` (new private fns; place near `translate_selected_section` ~:130)

**Interfaces:**
- Consumes: Task 2's `BTreeMap<u32, String>` bodies; Task 3's validators; existing `Numbered`/`NumberedMap` (`render.rs:33-48`).
- Produces:
  - `fn enrich_trigger_set(lines: &[&str], bodies: &BTreeMap<u32, String>) -> Vec<u32>` — nums whose excerpt contains a Latin token (len ≥ 4, alphabetic) absent (case-insensitive) from that line.
  - `fn build_enrich_prompt(pairs: &[(u32, &str)], bodies: &BTreeMap<u32, String>) -> String`
  - `fn sanitize_enriched(orig: &[(u32, String)], fresh: &str, numbered: &NumberedMap, bodies: &BTreeMap<u32, String>) -> (Vec<String>, usize, Vec<String>)` — returns (final lines, revert count, revert reasons).

- [ ] **Step 1: Failing tests** (all pure; prompt text asserted by substring, sanitizer by fixtures):

```rust
#[test]
fn trigger_fires_only_for_lines_missing_a_named_entity() {
    let bodies = bodies_map([(3, "Meta launched Muse Glimmer, a 30-billion-parameter model")]);
    let lines = ["- #3 Meta 發布開放權重模型", "- #4 輝達財報優於預期"];
    assert_eq!(enrich_trigger_set(&lines, &bodies), vec![3]);
}

#[test]
fn prompt_contains_rules_and_excerpt_blocks_only_for_body_items() { /* asserts the four verbatim rules below appear; #4 has no excerpt block */ }

#[test]
fn sanitizer_keeps_faithful_rewrite_reverts_hallucinated_token() {
    // fresh line for #3 with "Muse Glimmer" -> kept; with "Llama 5" -> reverted to orig, reason logged
}

#[test]
fn sanitizer_reverts_wrong_shape_and_missing_marker_lines() {
    // fresh output drops #4 entirely / unmarks a line -> those items revert to orig
}
```

- [ ] **Step 2: Run** → FAIL. - [ ] **Step 3: Implement.**

Prompt (verbatim; pairs = only triggered items; excerpt blocks only where `bodies` has the num):

```text
以下是已選定新聞的編號清單，部分項目附有該則的文章內文摘錄。規則：
1) 逐條輸出完整編號清單，格式與輸入完全一致（`- #N 標題`），一條都不能少。
2) 僅當摘錄明確寫出該條標題缺少的關鍵實體（如模型型號、產品名、機構名）時，才把該實體補進該條標題；沒有摘錄、或摘錄沒有點出標題缺少的實體的項目，必須逐字保留，一個字都不改。
3) 只能使用原標題與摘錄中明確出現的事實與名稱，嚴禁推測、換名或補充兩者都沒有的內容。
4) 維持繁體中文，風格與原標題一致。
```

Sanitizer rules, in order, per fresh line mapped to its `#N`: (a) marker set of fresh ⊇ marker set of triggered nums — missing nums revert to orig; (b) line must start with `- #N `; (c) `line_tokens_covered(line, title, excerpt_or_empty)`; (d) `has_cjk_2_in_head(line)`; (e) byte-equal (trim-end) to orig ⇒ keep orig unchanged and don't count as a change. Violation ⇒ keep orig line, push reason string. All-lines-reverted ⇒ caller treats as no-change (Task 5 traces it).

- [ ] **Step 4: Run** → PASS. - [ ] **Step 5: Commit** — `feat(news): enrich trigger + prompt + deterministic sanitizer (pure core)`

---

### Task 5: Wire `enrich_selected_section` into the two default paths

**Files:**
- Modify: `crates/news/src/summarize.rs` — new `fn enrich_selected_section(section_key: &str, lines: &[String], numbered: &NumberedMap, bodies: &BTreeMap<u32, String>, date_str: &str) -> Option<Vec<String>>`; call sites: AI happy path between `resolve_paywall_summaries` (~:715) and `attach_numbered_links` (~:728); default-section happy path between ~:571 and ~:581. AI path: call BEFORE `cache::put` (~:736) so enriched output is what gets cached.
- Modify: `crates/news/src/agent.rs` only if the existing runner isn't reusable as-is (prefer reuse — `nullclaw agent --isolated -m <prompt>` at `agent.rs:212-214`).

**Interfaces:**
- Consumes: Task 4 fns; `SharedCache` untouched; `log_trace` for events.
- Produces: trace events `enrich_trigger {section, n}`, `enrich_applied {section, changed, reverted}`, `enrich_noop {section}`, `enrich_failed {section, err}`; happy-path lines replaced only when `enrich_applied.changed > 0`.

- [ ] **Step 1: Failing test** — the runner is thin; the logic is Task 4's tested core. Add one wiring test if the crate already stubs nested-agent calls (check existing `summarize` tests for a runner seam); if none exists, assert via code review that `enrich_selected_section` contains no logic beyond: trigger → early-None on empty (trace noop) → prompt → run → parse → sanitize → None on runner error (trace failed) → Some(lines) when changed > 0, Some(orig) otherwise. **Do not invent a mock framework** (`docs/specs/2026-07-28-phase1-lessons.md`).
- [ ] **Step 2: Implement wiring.** `budget` note: pass the crate's existing LLM timeout (`LLM_TRANSLATION_TIMEOUT_SECS` is the closest existing knob — reuse it for the enrich call, no new timeout constant).
- [ ] **Step 3: Run** `cargo test -p news && cargo clippy --workspace --all-targets` → PASS, zero warnings.
- [ ] **Step 4: Commit** — `feat(news): enrich selected titles from precheck bodies on default sections`

---

### Task 6: P2 selection preference + cache variant bumps

**Files:**
- Modify: `crates/news/src/summarize.rs` — three selection-prompt builders (~:455-480 default, ~:643-657 AI, ~:949-961 custom) + custom cache variant string ~:918
- Modify: `crates/news/src/config.rs` — `AI_SUBSTAGE_CACHE_VARIANT` ~:39

- [ ] **Step 1:** Add this sentence to the exclusion/criteria block of all three prompts (verbatim):

```text
同一事件若有多則候選，優先挑選標題明確點出關鍵實體（公司、產品或型號名稱）的候選；若所有候選同樣籠統，仍照常挑選，不要因此跳過。
```

- [ ] **Step 2:** Bump `default_ai_clustered_v5_post_dedup` → `default_ai_clustered_v6_post_dedup`; `custom_topic_v3_dedup` → `custom_topic_v4_dedup`.
- [ ] **Step 3:** `grep -rn "v5_post_dedup\|custom_topic_v3" crates/news/src/` → zero stale references (fixture paths in tests may embed the variant string — update them in the same commit).
- [ ] **Step 4:** `cargo test --workspace` → PASS. - [ ] **Step 5: Commit** — `feat(news): prefer entity-naming titles at selection; bump cache variants`

---

### Task 7: Full gates, publish, post-deploy watch

- [ ] `cargo test --workspace && cargo clippy --workspace --all-targets && tools/lint-http.sh` — all green, zero warnings.
- [ ] Manual smoke (no delivery): run the binary with an unknown flag → exit 2; run `news --help`-equivalent locally with no `NULLCLAW_JOB_ID` (manual runs stay marker-clean).
- [ ] Publish: `tools/install-skill.sh news` (builds `--locked`, probes exit-2, publishes atomically); verify `readlink ~/.nullclaw/skills/news` still resolves into this repo (deploy-drift guard).
- [ ] Commit any lockfile changes — `chore(news): publish enriched-title build`.
- [ ] **Post-deploy watch (one week, owner-visible):** `news_run_timing` total_ms delta; `enrich_trigger/applied/noop/failed` counts; `cross_dedup_llm` kept/dropped ratio (enriched titles share more tokens ⇒ watch for false merges, bounded by `CROSS_DEDUP_MAX_DROP_RATIO = 0.40` at `config.rs:327-330`); promo drop count (P2 may bias toward press-release-style titles).

## Self-review notes

- Spec coverage: Kimi changes 1–6 all mapped (1→Task 4/5 revision design + header decision 1; 2→Task 2; 3→Task 3+4; 4→Task 1; 5→Task 6; 6→header decision 6). Codex D1–D4 confirmed; no open gaps.
- Type consistency: `Verdict::new` gains a 4th param in Task 1 and every later use site matches; `precheck_apply` 3-tuple introduced in Task 2, consumed in Task 5; bodies map type `BTreeMap<u32, String>` used identically in Tasks 2/4/5.
- Known v2 candidates (explicitly out of scope): custom-topic enrichment, paywall-replacement enrichment, `!language_ok` retry-path enrichment.
