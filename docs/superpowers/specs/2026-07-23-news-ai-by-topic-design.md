# Design: By-topic (theme-grouped) rendering for the AI news section

**Date:** 2026-07-23
**Skill:** `news` (Telegram AI/tech/general daily digest)
**Author:** Claude (brainstormed with user; direction reviewed by Codex, granularity/taxonomy by Grok, classifier mechanism by baicode/Qwen — all cross-verified against source)
**Status:** DRAFT — pending Codex spec review + user review before `writing-plans`.

## Problem / motivation (evidence-backed)

The AI section currently renders one flat, globally-deduplicated bullet list. Same-event
dedup (pre-translation `cluster()` → per-half LLM select → P3 cross-half vote ensemble) has
been pushed to an **LLM same-event JUDGMENT ceiling**: four metadata augmentations plus
embeddings were A/B-tested (N=20) on the hard cross-lingual case and none lifted recall past
the headline-only baseline's own 5–20% run-to-run noise (see
`docs/superpowers/specs/2026-07-14-news-cross-translate-dedup-design.md`, "Closed
investigations", and `news/SKILL.md`). Piling more tech onto global dedup has ~zero marginal
return for the hard cases.

This design does **not** try to improve dedup. It asks a different question — **is the daily
AI digest more readable when grouped by theme than as a flat list?** — and ships a
low-cost, falsifiable experiment to answer it.

### Reframe (locked before design)

By-topic is a **presentation/readability experiment**, applied strictly AFTER the existing
global P3 dedup, behind an `off/shadow/render` kill-switch, shadow-first. It is **not** a
dedup replacement, and it must never regress dedup.

## Non-goals (explicit)

- NOT a dedup improvement or replacement. P3 stays exactly as-is and stays global.
- NO growing the AI section beyond its current `pick: "5-8"` for this experiment (a denser
  section is a separate product decision with its own limit retune).
- NO multi-label classification (a story gets exactly one primary theme).
- NO LLM-dynamic / free-form theme names (fixed enum only).
- NO per-bucket dedup / no running the N=7/K=3 vote ensemble inside each theme.
- NOT touching `cluster()`, the half-split, translation, custom topics, or the weekly
  `ainews` project.

## Source constraints (verified — cite before trusting)

1. **AI section is small:** `DEFAULT_SECTION_SPECS["ai"]` has `"pick": "5-8"`, `"limit": 30`,
   and a `focus` string already listing sub-themes (研究突破/政策/產品發布/併購/國安監管).
   `news/scripts/run.py:83-90`. At 5–8 survivors, too many buckets → singleton sections.
2. **Theming MUST be post-P3.** `_parse_ai_blocks` fail-closes (returns `None`) on any line
   that is not a `- ` bullet, blank, or valid continuation. `news/scripts/run.py:2154-2168`.
   A theme heading inserted BEFORE P3 makes the parse fail → P3 passthrough → **cross-half
   dedup silently stops**. Theming is strictly post-P3.
3. **Exit-0 is not a repo-wide invariant.** Feeds-all-empty (`sys.exit(1)`,
   `run.py:3116-3127`) and AI-substage-exhausted (`_AiSubstageExhausted` → exit 1,
   `run.py:3145-3148`) legitimately exit non-zero. The classifier must be a refinement that
   **never owns an exit code** — it fails open to the current flat section and leaves the
   surrounding outcome unchanged.
4. **Sections are computed sequentially, then rendered.** `for key in section_keys:` runs
   ai → tech → general and stores each into `section_results[key]`
   (`run.py:2637-2676`, ai at :2643); a SEPARATE loop assembles headers+lines
   (`run.py:2688-2692`). So AI bullets are available for post-processing between the two
   loops. This split is what makes the classifier's insertion point (below) clean.
5. **LLM calls + budget:** `_run_nullclaw_agent` retries rc=124 once, budget-aware
   (`run.py:1081-1110`); `_skill_wallclock()` exposes remaining cron budget. Concurrent
   agent calls on one host contend (P3 caps in-flight at `CROSS_DEDUP_MAX_INFLIGHT=3`,
   `run.py:2308`), so the classifier runs sequentially, never concurrently with P3.

## Success criteria

- **Primary (hypothesis, must be A/B-validated):** the themed digest is more scannable /
  preferred over the flat list. Custom topics prove only that the renderer CAN show titled
  sections, not that it is more readable — so readability is a claim to measure, not assume.
- **Hard guardrail (non-negotiable):** no dedup regression. In `shadow` the delivered body is
  **byte-equal** to the current flat digest; classification changes nothing user-visible.
  No increase in false merges (theming never merges/drops bullets — it only groups them).
- **Secondary guardrails:** `其他` share, bucket balance, cross-theme duplicate suspects,
  important-story coverage, added wall-clock, malformed/fallback rate.

### Shadow is the go/no-go gate (not just a safety step)

At 5–8 bullets with 4 themes, many days theming collapses to near-flat (each theme a
singleton). That is a *correct* degenerate outcome, but it also means the readability payoff
is uncertain. `shadow` mode MEASURES whether theming fires often enough to be worth enabling.
Only flip to `render` if, over several days:

- `其他` share median ≲ 25–30%;
- on days with ≥6 AI bullets, ≥1 theme has ≥2 stories at least ~half the time;
- shadow body is byte-equal to the flat control on every run (proves zero dedup regression).

If those thresholds are not met, the experiment concludes at `shadow` — theming is not worth
rendering — and no render code path is enabled.

## Taxonomy (fixed enum, single primary theme)

Four primary themes + `其他`. Labels in Traditional Chinese:

| Theme | Belongs here (single primary) |
|-------|-------------------------------|
| **產品發布** | Concrete ship/GA/API/launch of a user-facing or API product/feature. |
| **研究突破** | Papers, benchmarks, capability/science claims WITHOUT a clear product-ship frame. |
| **產業資本** | M&A, funding, IPO/earnings-as-capital, strategic deals, market structure. |
| **政策監管** | Law, regulation, government action, export controls, 國安-as-state-power (merges 政策+國安+監管 — they blur in RSS). |
| **其他** | No clean primary fit, or genuinely multi-aspect with no dominant frame (leadership drama, lawsuits w/o reg outcome, outages, rumor, soft analysis). |

**Single-label tie-break (first-match priority, NOT "best fit"):**
政策監管 → 產業資本 → 產品發布 → 研究突破 → 其他.
Worked edges: "OpenAI ships GPT-X in ChatGPT" → 產品發布; "paper claims SOTA, no product"
→ 研究突破; "US export curb on AI chips" → 政策監管; "Anthropic raises $X" → 產業資本;
"CEO resigns / outage / vague trend piece" → 其他.

Rationale for 4 (not 5) primaries: the `focus` string lists 5 ideas, but 5 primaries over
5–8 survivors averages ~1 story/theme → adaptive rendering collapses everything → theming
adds no structure. Four is the granularity where a busy day clusters into multi-story buckets
and a quiet day honestly degrades to flat.

## Architecture

### Insertion point (verified)

Run classification + theme rendering as a **post-processing step on `section_results["ai"]`,
AFTER the compute loop (`run.py:2637-2676`) and BEFORE the render loop
(`run.py:2688-2692`)** — NOT inside `_summarize_default_ai_substaged`. Placing it inside the
AI iteration would delay tech/general and eat their wall-clock budget; placing it after all
sections are computed means it squeezes no section and its budget-gate need only watch the
kill window. (This refinement over "right after P3" was surfaced by baicode and verified
against the compute/render split.)

### The classifier

- **One cheap LLM call** over the 5–8 post-P3 AI headlines, returning one enum label per
  headline. Keyword rules are rejected as the primary mechanism: on translated CJK titles
  they are ~50–75% accurate and drift silently as Google-News translation varies ("launches"
  → 發表/推出/上線…), i.e. they trade the LLM's rare timeout for a chronic, silent
  misclassification. (Keyword pre-filtering for an unambiguous class, e.g. strong regulatory
  terms → 政策監管, is an OPTIONAL later refinement, never the main path.)
- **Separate call — do NOT fold into P3.** P3's prompt/response is a pair-vote contract
  (`_cross_dedup_ai`, `run.py:2392+`); folding classification in would break it.
- **Params:** dedicated short timeout `CLASSIFIER_TIMEOUT_SECS ≈ 8` (5–8 short titles
  classify in ~1–3s on an idle host; 8s is generous). **No retry** (it is a refinement, not a
  critical path — do not spend double wall-clock). **Budget-gated** via `_skill_wallclock()`:
  if remaining cron budget is too low, skip classification and render flat.
- **Validation:** response must yield exactly one valid enum label per input headline. Reject
  the WHOLE response (→ flat) on: nonzero rc / empty stdout / JSON parse fail / label count ≠
  headline count / any label outside the enum.

### The renderer (adaptive)

```
labels = classify(ai_bullets)            # post-compute, pre-render
for theme in [產品發布, 研究突破, 產業資本, 政策監管]:   # fixed order
    if count(theme) >= 2:
        emit heading + that theme's bullets   # within-theme = post-P3 order
    else:
        queue theme's singleton bullets to the flat tail
if count(其他) >= 2:
    emit 其他 heading + bullets      # 其他 always last among headed groups
elif count(其他) == 1:
    append to flat tail
emit flat tail (unheaded) if non-empty   # tail keeps post-P3 relative order
```

- **Cross-theme order:** fixed (產品→研究→產業→政策→其他), never by today's bucket size
  (avoids day-to-day thrash).
- **Within-theme order:** preserve post-P3 order (the significance-ish ranking already paid
  for). No re-sort by source/recency, no second significance LLM.
- **Every theme a singleton → output is today's flat list (zero headings).** This is a
  successful degenerate case, not a bug.
- **Layout folding ≠ reclassification:** a singleton dropped to the tail keeps its true
  assigned label in the trace (`theme_assigned` vs `theme_rendered`) so shadow distribution
  metrics are not poisoned.

### Kill-switch & modes

`NEWS_AI_THEME` (call-time env read, honors `~/.nullclaw/.env`), default **`off`**:

- **`off`** (default) — skip the classifier entirely; identical to today. Deploying this
  feature changes NOTHING until the operator explicitly opts in — no extra LLM call, no added
  wall-clock, no risk on the production cron.
- **`shadow`** — classify, compute the themed layout, TRACE everything, but deliver the
  **byte-equal flat body**. Pure measurement window, zero user-visible change. Operator turns
  this on when ready to gather the go/no-go data.
- **`render`** — deliver the adaptive themed layout. Enabled only after shadow thresholds met.

> Decision note (Codex/Grok split, resolved by author): Grok proposed default `shadow`; Codex
> proposed default `off`. Chosen `off` — deploying the code should be a behavioral no-op that
> adds no LLM call until the operator opts into the measurement window. Flagged for Codex spec
> review.

### Failure semantics (binary — no middle state)

Whole-classifier failure → **flat section unchanged** (= `off` behavior) on: rc=124 /
nonzero rc / empty stdout / JSON parse fail / label-count mismatch / illegal label / any
exception / insufficient budget. Never partial ("3 themed + 2 flat"). A missing/illegal label
for one story means the whole response is invalid → flat (a single story is only labeled
`其他` when the LLM SUCCEEDS and actively assigns 其他). This keeps shadow byte-equal
verification simple.

### Trace / telemetry

Emit `ai_theme` with: mode (off/shadow/render), per-story `theme_assigned`, `theme_rendered`
(heading vs tail), `其他` share, bucket balance, cross-theme duplicate suspects, classifier
`ok`/`error`/`elapsed_ms`, and whether the budget-gate skipped it. In shadow, also assert the
themed vs flat body relationship for the byte-equal invariant.

## Later optimization (out of scope for v1)

If `render` wins on readability, fold the single-primary theme code into the existing
half-select LLM responses (Codex's "Approach B") to remove the extra call's wall-clock. That
changes the failure-sensitive select contract and requires an `AI_SUBSTAGE_CACHE_VARIANT`
bump (`run.py:40, 2012`), so it is deliberately deferred until v1 proves value.

## Testing (TDD — RED first)

Real fixtures, LLM stubbed (monkeypatch the classify agent call to return canned labels):

1. **Classify parse/validate:** valid one-label-per-headline; reject each of — non-JSON,
   label-count mismatch, illegal label, empty stdout, nonzero rc → whole-response rejection.
2. **Adaptive render — clustered day:** ≥2 in a theme → heading emitted; within-theme order =
   input order.
3. **Adaptive render — all singletons:** output byte-equals the flat input (zero headings).
4. **Singleton folding:** a lone theme and a lone `其他` go to the unheaded tail, in post-P3
   order; assigned labels still recorded in trace.
5. **Fixed cross-theme order** regardless of bucket sizes; `其他` last.
6. **Kill-switch off:** classifier not called; output identical to flat.
7. **Kill-switch shadow:** classifier called, body delivered = flat (byte-equal), trace has
   labels.
8. **Classifier failure safe:** rc=124 / illegal response / exception / budget-gate skip →
   flat section unchanged, exit code unaffected.
9. **Budget-gate:** low `_skill_wallclock()` remaining → classifier skipped, flat rendered.
10. **Insertion point:** classification runs on `section_results["ai"]` after all sections
    computed; tech/general sections unaffected in content and not delayed by classify in the
    `off` path.
11. **Trace shape:** `ai_theme` with mode + theme_assigned/theme_rendered + 其他 share.

Tests in `news/scripts/test_run.py`. Existing suite must stay green.

## Companion doc fixes (independent of this feature; do alongside)

1. `news/SKILL.md:107` says the hard same-event case is an "資訊限制" (information limitation);
   the newer experiment-specific conclusion is that it is a JUDGMENT limit (the decoded slug
   supplied the entity "kimi" and the model still would not merge). Correct the wording to
   "judgment 限制" for consistency with the closed-investigation addendum and memory.
2. The cross-dedup addendum states embedding "gave no lift" without the N=20 rigor recorded
   for the other four augmentations. Soften to "embedding also tested; no lift observed
   (sample size not held to the N=20 of the four metadata augmentations)".

## Out of scope

- Any dedup change; growing the AI section; multi-label; dynamic themes; per-bucket ensemble.
- Custom-topic theming; tech/general theming; `ainews`.
- Approach B (fold theme code into half-select calls) — deferred until v1 proves value.
