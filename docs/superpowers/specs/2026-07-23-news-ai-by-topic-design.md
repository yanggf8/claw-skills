# Design: By-topic (theme-grouped) rendering for the AI news section

**Date:** 2026-07-23
**Skill:** `news` (Telegram AI/tech/general daily digest)
**Author:** Claude (brainstormed with user; direction reviewed by Codex, granularity/taxonomy by Grok, classifier mechanism by baicode/Qwen; **spec adversarially reviewed by Codex and every finding cross-verified against source** — see "Codex spec review" at the end)
**Status:** DRAFT v2 — Codex findings resolved; pending user review before `writing-plans`.

## Problem / motivation (evidence-backed)

The AI section currently renders one flat, globally-deduplicated bullet list. Same-event
dedup (pre-translation `cluster()` → per-half LLM select → P3 cross-half vote ensemble) has
been pushed to an **LLM same-event JUDGMENT ceiling**: four metadata augmentations were
A/B-tested (N=20) on the hard cross-lingual case and none lifted recall past the headline-only
baseline's own 5–20% run-to-run noise; embedding was also tested (no lift observed, sample
size not held to the same N=20 rigor). See
`docs/superpowers/specs/2026-07-14-news-cross-translate-dedup-design.md` "Closed
investigations" and `news/SKILL.md`. Piling more tech onto global dedup has ~zero marginal
return for the hard cases.

This design does **not** try to improve dedup. It asks a different question — **is the daily
AI digest more readable when grouped by theme than as a flat list?** — and ships a low-cost,
falsifiable experiment to answer it.

### Reframe (locked before design)

By-topic is a **presentation/readability experiment**, applied strictly AFTER the existing
global P3 dedup, behind an `off/shadow/render` kill-switch, shadow-first. It is **not** a
dedup replacement, and it must never regress dedup or drop a story.

## Non-goals (explicit)

- NOT a dedup improvement or replacement. P3 stays exactly as-is and stays global.
- NO changing AI selection size (the proportional `pick_count`, below) as part of this feature.
- NO multi-label classification (one primary theme per story).
- NO LLM-dynamic / free-form theme names (fixed enum only).
- NO per-bucket dedup / no running the N=7/K=3 vote ensemble inside each theme.
- NOT touching `cluster()`, the half-split, translation, custom topics, or the weekly
  `ainews` project.

## Source constraints (verified against `news/scripts/run.py`)

1. **Post-P3 AI cardinality is `K`, NOT a fixed 5–8.** The substaged AI path ignores the
   `pick: "5-8"` config; each half selects `pick_count = max(2, len(sub_items)//3)`
   (`run.py:2030`), the two halves concatenate (`:2612-2614`), then P3 dedups (`:2620`). So
   the delivered block count `K ≈ n/3` of the post-cluster item count — realistically **~8–15**,
   occasionally more. **The classifier must operate on the ACTUAL parsed block count `K`,
   never an assumed 5–8**, and cap its input (fail flat above `THEME_MAX_BLOCKS`, e.g. 20).
2. **Theming MUST be post-P3.** `_parse_ai_blocks` fail-closes (returns `None`) on any line
   that is not a `- ` bullet, blank, or valid continuation (`run.py:2154-2168`). A theme
   heading inserted BEFORE P3 makes the parse fail → P3 passthrough → **cross-half dedup
   silently stops**.
3. **Exit-0 is not a repo-wide invariant.** Feeds-all-empty (`sys.exit(1)`, `run.py:3127`),
   `_AiSubstageExhausted` (exit 1, `:3151`), delivery failure and uncaught exceptions
   (`:3173`) all legitimately terminate non-zero. The classifier must be a refinement that
   **never owns an exit code** and never converts a would-be-successful run into a failure.
4. **Sections computed, then rendered — with an alert block between.** `for key in
   section_keys:` computes ai→tech→general into `section_results[key]`
   (`run.py:2637-2676`); then a `degraded_sections` alert block (`:2678-2686`); then a
   separate render loop assembles headers+lines (`:2688-2692`); then global paywall footer
   (`:2699-2701`) and `_trim_digest_links` (`:2703`). The theming post-processor plugs in
   **after the degraded alert block (:2686) and before the render loop (:2688)**, replacing
   only `section_results["ai"]`.
5. **`_trim_digest_links` is length-sensitive and section-aware (HAZARD).** It only fires when
   visible text > 4000 chars (`run.py:1659`) and detects the AI section by "AI 人工智慧" in a
   line, ending it on the next line that `startswith("**")` (`:1665-1668`). Downstream
   `_trim_lines_to_limit` can DELETE whole bullets + their paywall continuations (`:1580-1588`).
6. **LLM/budget primitives.** `_run_nullclaw_agent` retries rc=124 once (`:1081-1110`);
   `_run_nullclaw_agent_once` (`:1113`) is the no-retry entry. `_skill_wallclock` is
   **trace-only, no reserve, returns `None` for remaining when start ts is absent**
   (`:1055-1078`) — NOT usable as a budget gate; `_llm_retry_budget_secs` (`:1024-1052`) is
   the reserve-aware pattern to copy. P3 abandons daemon workers at its deadline and returns
   while they are still alive (`:2436-2447`, test `test_run.py:2308`) — a later step CAN
   overlap a lingering P3 subprocess.

## Success criteria

- **Primary (hypothesis, must be A/B-validated):** the themed digest is more scannable /
  preferred over the flat list. Readability is a claim to measure, not assume.
- **Hard guardrail (non-negotiable):** no dedup regression and no dropped story. In `shadow`
  the delivered body is **byte-equal to the off-mode digest** (measured on the FULL digest
  after `_trim_digest_links` and the job-id append, not just the AI section). Theming only
  regroups blocks; it never merges, drops, or rewrites one.
- **Secondary guardrails:** `其他` share, bucket balance, coverage, added wall-clock,
  malformed/fail-open rate.

### Shadow is the go/no-go gate

`shadow` MEASURES whether theming fires often enough to be worth `render`. Flip to `render`
only if, over several days: `其他` share median ≲ 25–30%; on days with `K ≥ 8`, ≥1 theme has
≥2 stories most of the time; and shadow body is byte-equal to the off-mode control on every
run. If not met, the experiment concludes at `shadow` and no render path is enabled.

## Taxonomy (fixed enum, single primary theme)

Four primary themes + `其他`, Traditional Chinese labels:

| Theme | Belongs here (single primary) |
|-------|-------------------------------|
| **產品發布** | Concrete ship/GA/API/launch of a user-facing or API product/feature; enterprise deployment/adoption of a shipped product. |
| **研究突破** | Papers, benchmarks, capability/science claims, **and technical AI-safety/alignment reports**, without a clear product-ship frame. |
| **產業資本** | M&A, funding, IPO/earnings-as-capital, strategic deals/partnerships, market structure. |
| **政策監管** | Law, regulation, government action, export controls, 國安-as-state-power (merges 政策+國安+監管). |
| **其他** | No dominant news peg among 1–4 (leadership drama, lawsuits w/o reg outcome, outages, rumor, soft trend pieces). |

**Classification rule — DOMINANT NEWS PEG, not first-match.** Assign the theme of the
headline's *dominant* news peg; apply a priority order ONLY to break a genuine tie
(政策監管 → 產業資本 → 產品發布 → 研究突破 → 其他). This avoids the failure Codex flagged
where a strict first-match labels every government-backed investment or regulated product
launch as 政策 even when capital/product is the real peg. Worked edges: "OpenAI ships GPT-X
in ChatGPT" → 產品發布; "gov-backed $Xbn AI fund" → 產業資本 (peg is capital) unless the peg
is the regulation itself; "US export curb on AI chips" → 政策監管; "paper claims SOTA, no
product" → 研究突破; "safety eval finds jailbreak" → 研究突破; "CEO resigns / outage" → 其他.

**Enterprise adoption and AI-safety reports are explicitly mapped** (to 產品發布 and 研究突破
respectively) because the AI remit includes them (`SKILL.md:63,65`) and they would otherwise
inflate `其他`.

Rationale for 4 primaries: at the corrected cardinality (`K ≈ 8–15`), four themes average
~2–4 stories/bucket on a normal day — enough structure to help, few enough that adaptive
rendering rarely degenerates. Five primaries risks thin buckets; three loses useful
structure.

## Architecture

### Insertion point (verified)

A single post-processor `_theme_ai_section(...)` runs **between the degraded-section alert
block (`run.py:2686`) and the render loop (`:2688`)**, replacing `section_results["ai"]`.

**Skip theming entirely (return the AI lines untouched) when ANY of:**
- `NEWS_AI_THEME` is `off` or an **unrecognized value** (unknown → `off`, never `render`);
- `"ai"` is in `degraded_sections` (fallback lines that never went through P3);
- the AI lines are the no-news placeholder `["- 今日無相關新聞"]` (`:2618`);
- `_parse_ai_blocks` returns `None` or fewer than 2 blocks;
- block count `K > THEME_MAX_BLOCKS`;
- the budget gate (below) says no.

### The classifier (block-based, one no-retry call)

- Reuse `_parse_ai_blocks` to get atomic **story blocks** (each a `start:end` line slice; a
  free-replacement paywall story is 2 lines / 1 block). Classify **one block per input
  object**, sending `headline` and optional `original_headline` — never a physical line.
- **One cheap LLM call** via `_run_nullclaw_agent_once` (`run.py:1113`) — the no-retry entry;
  a rc=124 retry is not worth doubling wall-clock for a refinement. Returns one enum label
  per block. Keyword rules are NOT the primary mechanism: on translated CJK titles they drift
  silently as Google-News translation varies; a rules-only path trades a rare timeout for
  chronic misclassification. (Keyword pre-tagging of an unambiguous class is an OPTIONAL later
  refinement.)
- **Do NOT fold into P3** — P3's prompt/response is a pair-vote contract (`_cross_dedup_ai`,
  `:2392`); folding classification in would break it.
- **Params:** dedicated short timeout `CLASSIFIER_TIMEOUT_SECS` (benchmark against realistic
  `K≈8–20` short headlines, not 5–8; start ~10s). No retry (uses `_once`).
- **Validation → whole-response reject (→ flat):** nonzero rc / empty stdout / JSON parse
  fail / label count ≠ block count / any label outside the enum / duplicate or missing block
  ids (if ids are used).

### Budget gate (dedicated helper, not `_skill_wallclock`)

Add `_theme_budget_ok()` modeled on `_llm_retry_budget_secs`: it must reserve
`CLASSIFIER_TIMEOUT_SECS` **plus a delivery reserve** (Telegram spends up to 15s/attempt
`lib/telegram.py:19`, delivery ~1s to exit `lib/delivery.py:95,101`). When a cron timeout is
configured but a reliable remaining time is unavailable (no `NULLCLAW_SKILL_STARTED`), **skip**
(fail flat). Because the post-processor runs after all sections, the gate need only protect
finalization+delivery, not tech/general.

### The renderer (adaptive, block-atomic, non-`**` headings)

```
blocks = _parse_ai_blocks(ai_lines); labels = classify(blocks)
groups = {theme: [blocks with that label] preserving post-P3 order}
if no theme reaches >=2:  return ai_lines UNCHANGED   # exact bytes, incl. blanks
out = []
for theme in [產品發布, 研究突破, 產業資本, 政策監管]:      # fixed order
    if len(groups[theme]) >= 2:
        out += [THEME_HEADING(theme)] + [lines[b.start:b.end] for b in groups[theme]]
    else: tail += singleton block slices
if len(groups[其他]) >= 2: out += [THEME_HEADING(其他)] + 其他 block slices   # 其他 last
else: tail += 其他 singleton slices
out += tail (unheaded, post-P3 order)
```

- **Heading format MUST NOT start with `**`** (e.g. use `▸ 產品發布` or an emoji prefix) —
  a `**`-prefixed line makes `_trim_digest_links` think the AI section ended and strip links
  from the rest of the AI bullets (`run.py:1667`). Add a test asserting the chosen format does
  not set `in_ai = False`.
- **Move `lines[start:end]` slices intact** — never reconstruct a headline/link, never split a
  paywall pair across a heading.
- **Length/trim guard (no-drop invariant):** headings add bytes and can push the digest over
  the 4000-char `_trim_digest_links` threshold, which can strip AI links and (via
  `_trim_lines_to_limit`) drop whole blocks — and the paywall footer is counted BEFORE
  trimming (`:2699` vs `:2703`), so a trim-drop leaves a stale count. Therefore: if the
  themed full digest (headings + footer) would exceed the trim threshold, **render unthemed
  (flat)** for that run rather than risk a dropped story or stale footer. (This keeps the
  no-drop guarantee absolute; a later version may refine finalization ordering.)

### Kill-switch, modes, failure semantics

`NEWS_AI_THEME` (call-time env read; `load_env()` runs before summarization `run.py:3057`, so
`~/.nullclaw/.env` is honored), default **`off`**:

- **`off`** (default) — post-processor not invoked; identical to today. Deploying changes
  nothing until the operator opts in (no LLM call, no latency, no risk).
- **`shadow`** — classify + compute the themed layout from a COPY, TRACE everything, but
  deliver the untouched flat AI lines. Pure measurement.
- **`render`** — deliver the adaptive themed layout (only after shadow thresholds met).

> Decision note (Codex/Grok split, resolved): Grok proposed default `shadow`, Codex `off`.
> Chosen `off` (Codex concurred): shadow still spends an LLM call, trace writes, latency, and
> possible P3-subprocess contention, so deploying should be a no-op until opt-in.

**Total fail-open.** The entire post-processor (mode parse, block parse, prompt build,
subprocess, validation, render, trace) is wrapped in ONE top-level `try/except` that returns
the untouched flat AI lines on ANY exception — because the insertion point is outside the
section loop's handler and an escaped exception would otherwise reach `main`'s exit-1 path
(`run.py:3173`). Fail-open triggers: rc=124 / nonzero rc / empty / parse fail / label
mismatch / illegal label / budget skip / any exception. **No partial render** ("3 themed + 2
flat" never happens); `其他` appears only when the LLM SUCCEEDS and actively assigns it.

### Trace / telemetry

Emit `ai_theme` with: mode, `K`, per-block `theme_assigned` and `theme_rendered`
(heading vs tail), `其他` share, bucket balance, classifier `ok`/`error`/`elapsed_ms`, and
budget-gate/length-guard skip reasons. **Drop the earlier "cross-theme duplicate suspects"
field** — P3 returns only rendered lines and its votes/headlines live in a separate trace
(`run.py:2499,2528`), so the post-processor has no structured near-miss data to compute it
without re-plumbing P3. (If wanted later, derive it from P3's own trace, not here.)

## Later optimization (out of scope for v1)

If `render` wins, fold the single theme code into the existing half-select LLM responses
(Codex's "Approach B") to remove the extra call; that changes the failure-sensitive select
contract and needs an `AI_SUBSTAGE_CACHE_VARIANT` bump (`run.py:40,2012`), so it is deferred.

## Testing (TDD — RED first). LLM stubbed; existing suite stays green.

1. Classify parse/validate: valid one-label-per-block; reject each of non-JSON, label-count
   mismatch, illegal label, empty, nonzero rc, duplicate/missing ids → whole-response reject.
2. Adaptive render — clustered day (`K≈12`, ≥2 in a theme) → heading; within-theme order =
   post-P3 order.
3. Adaptive render — all singletons → output **byte-equals** the flat input (zero headings).
4. Singleton folding: lone theme + lone `其他` → unheaded tail in post-P3 order; assigned
   labels still in trace.
5. Fixed cross-theme order regardless of bucket sizes; `其他` last.
6. **Atomic two-line paywall block:** classified as one story; no heading between parent and
   continuation; moved intact; global paywall count unchanged.
7. **Heading format:** chosen heading does NOT trip `_trim_digest_links`'s `in_ai` reset (no
   `**` prefix); AI links survive a >4000 digest.
8. **Length/trim guard:** a themed digest that would cross the 4000 threshold → renders flat
   (no dropped block, no stale footer).
9. Kill-switch `off`: not invoked, identical output. Unknown mode value → treated as `off`.
10. Kill-switch `shadow`: classifier called; delivered FULL digest (after `_trim_digest_links`
    + job-id append) byte-equals the off-mode golden; trace has labels.
11. Fail-open: rc=124 (assert exactly ONE agent call via `_once`) / illegal response /
    renderer exception / trace exception / budget skip → flat, exit code unaffected.
12. Skip paths: AI in `degraded_sections`; no-news placeholder; `<2` blocks; `K > THEME_MAX_BLOCKS`.
13. Insertion point: runs on `section_results["ai"]` after all sections computed; tech/general
    content unaffected.
14. P3-abandoned-worker overlap: theming still fails open / completes without corrupting output.
15. Trace shape: `ai_theme` with mode + K + assigned/rendered + `其他` share.

## Companion doc fixes (independent; do alongside)

1. `news/SKILL.md:107` "資訊限制" → "judgment 限制" (consistent with the closed-investigation
   addendum: the decoded slug supplied the entity and the model still would not merge).
2. Soften the cross-dedup addendum's embedding claim to "also tested; no lift observed (sample
   size not held to the N=20 of the four metadata augmentations)".

## Out of scope

- Any dedup change; changing AI selection size; multi-label; dynamic themes; per-bucket
  ensemble; custom-topic/tech/general theming; `ainews`; Approach B (deferred).

---

# Codex spec review (2026-07-23) — findings verified against source & resolved

All findings below were re-checked line-by-line against `run.py`/`test_run.py` before folding
in; every one held.

- **#2 (High) — "5–8" premise wrong.** Verified `pick_count = max(2, len//3)` per half
  (`:2030`); real `K≈8–15`. → Rewrote §Source-constraint 1, taxonomy rationale, and shadow
  thresholds around actual `K`; added `THEME_MAX_BLOCKS`.
- **#3 (High) — insertion clean only on the successful path.** Verified degraded fallback
  (`:2662`) and no-news placeholder (`:2618`) bypass P3; alert block sits between the loops
  (`:2678`). → Insert after `:2686`; skip degraded/placeholder/parse-fail/`<2`.
- **#4 (High) — block atomicity + trim/paywall/`**`-heading hazards.** Verified
  `_trim_digest_links` `in_ai` reset on `**` (`:1667`), 4000 threshold (`:1659`),
  block-drop (`:1580-1588`), footer-before-trim (`:2699` vs `:2703`). → Block-atomic renderer,
  non-`**` headings, length/trim guard → render flat rather than risk a drop.
- **#5 (High) — fail-open/budget.** Verified `_skill_wallclock` is trace-only/no-reserve
  (`:1055-1078`) and `_run_nullclaw_agent_once` exists (`:1113`). → Dedicated `_theme_budget_ok`
  with delivery reserve; `_once` for no-retry; one top-level try/except; unknown mode → off.
- **#6 (Med) — "never concurrent with P3" false.** Verified P3 abandons live daemon workers
  (`:2436-2447`, test `:2308`). → Dropped the guarantee; short-timeout + fail-open absorb it.
- **#7 (Med) — shadow invariant.** → Compare FULL post-trim + job-id digest to an off-mode
  golden (not themed==flat).
- **#8 (Med) — taxonomy.** → Dominant-peg rule (priority only for ties); enterprise + safety
  explicitly mapped to curb `其他`.
- **#9 (Med) — tests/telemetry.** → Added atomic-paywall, heading-format, length-guard,
  degraded/placeholder, one-shot-count, invalid-mode, abandoned-worker tests; dropped the
  unsupported cross-theme-duplicate metric.
- **#10 — docs/default.** → Softened embedding/keyword/latency claims to hypotheses;
  default `off` confirmed by Codex.
