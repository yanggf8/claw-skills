# Design: LLM cross-half same-event dedup for the AI section (news skill)

**Date:** 2026-07-14
**Skill:** `news` (Telegram AI/tech/general daily digest)
**Author:** Claude (brainstormed with user)
**Status:** v3 — LLM approach, hardened per two rounds of Codex review (v1 deterministic-overlap rejected round 1; LLM approach confirmed sound-with-fixes round 2, fixes folded in below)

## Problem (root-caused, evidence-backed)

On 2026-07-14 the AI section shipped the same news story twice:
- **#5** 「祖克柏豪賭 AI：單座資料中心上看 2,500 億美元」
- **#9** 「Meta 路易斯安那資料中心投資將達 500 億美元，受惠於優渥稅務優惠」

Verified (WebSearch + DataCenterDynamics / Bloomberg-via-Yahoo): both are the **same
event** — Meta's Louisiana **Hyperion** campus — from different outlets with different
investment estimates. Same story, different hook + number.

Why the existing three dedup layers missed it (confirmed by re-running `cluster()`,
reading traces `skill-75e98cbb` for 2026-07-14, and Codex source review):

1. **`cluster()` runs on PRE-translation titles** (`run.py:2104`), before
   `_translate_selected_section`. The two RSS originals shared too few tokens to cluster.
2. Both survived to the 47-item list and were split by the **Level-2 half-cut**
   (`mid = n//2`, `halves=[(0,mid),(mid,n)]`, `run.py:2117`): #5 in front half (cache
   `000-023`), #9 in back half (`023-047`). **Each half is an independent LLM call.**
3. `_post_dedup_selected_summary` (P2, `run.py:2032`) runs **per-half** on
   **pre-translation** tokens — cannot catch a cross-half, post-translation duplicate.

## Why NOT deterministic token overlap (v1 rejected)

The first design proposed a deterministic `_topic_words` overlap≥4 pass on the merged
translated titles. **Codex review + independent verification proved it misaimed:**

- **False-merge, verified:** `_topic_words("Google 投資 100 億美元興建日本資料中心")` ∩
  `_topic_words("Microsoft 投資 50 億美元擴建德國資料中心")` = `{投資,億美,美元,資料,料中,中心}`
  = **6** — identical overlap to the real #5#9 duplicate (also 6). Different companies,
  countries, events. **overlap≥N cannot distinguish same-event from same-theme** on
  translated CJK titles; the shared tokens are generic ("資料中心投資" vocabulary) +
  numeric noise (`500` appears because `2,500` splits to `2`+`500`). Raising the floor
  can't fix it: 6 admits the false case, 7 misses the real one.
- **Rendered-line hazard:** the merge point holds link-attached lines (`with_links`,
  `run.py:2073`), so tokenizing them makes `https/news.google.com/rss/articles` shared
  words → any two linked bullets falsely merge.

Deterministic overlap is why the user asked for LLM dedup from the start. It's needed.

## Approach: an LLM same-event grouping pass (judgment only, program cuts)

After both AI halves are selected, translated, and merged into one list, add a pass that:

1. Extracts the **clean headline text** of each merged AI bullet (strip bullet syntax,
   the Markdown link, and any `⚠️ 付費牆` / continuation line — see "Rendered-line
   safety") into a numbered list `#1..#K`.
2. Sends that clean numbered list to ONE LLM call asking: **"which of these bullets cover
   the SAME news event? Return groups of numbers; each group is one event."** The LLM
   returns groups only — **it does NOT rewrite text, reorder, or touch links.**
3. Program applies the groups: for each same-event group, **keep one bullet, drop the
   rest** (with their continuation lines, atomically). Keep the group's **earliest merged
   bullet** by default (front-half then back-half order), OR let the LLM name which member
   to keep within each group (decided below).

This is the same shape as the existing DEDUP_RULES layer (LLM judges same-event), but
applied **cross-half on the merged, translated list** — the gap the three existing layers
leave open. LLM semantic judgment (not token overlap) is what distinguishes
"Google 日本 vs Microsoft 德國" (keep both) from "祖克柏 2500億 vs Meta 路易斯安那 500億"
(same Hyperion event, merge).

### Rendered-line safety (fixes Codex round-1 Critical #1)

The LLM sees ONLY clean headline strings, never rendered lines. Concretely:
- Parse the merged AI section into atomic **story blocks** first: a headline bullet plus
  its optional paywall-replacement continuation line. A continuation is attributable ONLY
  when it immediately follows a parent AND begins with the exact `PAYWALL_CONT_PREFIX`
  (`run.py:1275`). **Fail closed:** parse only the exact parent/continuation forms; retain
  every original line byte-for-byte; no-op the whole pass on an orphan continuation, an
  unexpected non-bullet line (e.g. a stray `以下是結果：`), or any ambiguous input. The
  cleaned strings fed to the LLM are a SEPARATE copy — never reconstruct delivery lines
  from them.
- For a **paywall-replacement block** (`run.py:1312`) there are TWO headlines: the free
  replacement on the parent line and the original selected headline on the continuation.
  Feed the LLM structured metadata per block so it can both match events and pick the
  representative WITHOUT a URL (so no S4 free-source-ranking violation):
  `{"id":N, "headline":"<parent clean>", "original_headline":"<continuation clean or null>",
  "access":"free_replacement"|"paywalled"|"normal"}`.
- When dropping a block, drop the whole block (parent + continuation) atomically.
- **Do NOT touch the paywall footer.** Confirmed from source (`run.py:2251`): the footer is
  generated globally at digest-assembly time by `digest.count(PAYWALL_NOTE)`, NOT per
  section. So the helper only removes the dropped block's lines (its `PAYWALL_NOTE` marker
  goes with it); the global count then stays correct automatically. Generating an AI-only
  footer would produce a second, wrong footer.

### LLM response contract & validation (fixes Codex round-2 HIGH — transactional validation)

The LLM call goes through `_run_nullclaw_agent` (`run.py:1045`), which auto-retries rc=124
ONCE. Mirror the existing AI-substage guard (`run.py:2023-2029`): check return code BEFORE
trusting stdout. Response is a strict JSON schema, e.g.
`{"groups": [{"members": [4,9], "keep": 4}, ...]}`. **Reject the ENTIRE response (→ passthrough
no-op) if ANY of:** rc != 0 / empty stdout / JSON parse fails / any member not an integer in
`1..K` / a group has duplicate members / a group has < 2 distinct members / two groups overlap
/ `keep` is not exactly one member of its own group / applying all groups would leave zero
blocks. No partial application of an invalid response — all-or-nothing.

### Catastrophic-collapse circuit breaker (fixes Codex round-2 HIGH — underfill)

"Non-empty" alone is not enough: a structurally-valid grouping could collapse 8 unrelated
stories into 1. AI target is 5–8 (`run.py:82`). **Whole-pass no-op** (keep original merged
section, emit a rejection trace) if applying the groups would drop the block count below
`min(before, 5)` OR remove more than a defined fraction (e.g. > 50%) of blocks. Same-event
merge after pre-cluster + per-half P2 should only remove a handful of true dups, so a
dramatic reduction is a signal the LLM over-merged same-theme stories.

### Precision-first prompt (fixes Codex round-2 MEDIUM — guardrails)

Reuse the existing same-event-vs-same-theme distinction (`run.py:123` `DEDUP_RULES`).
The grouping prompt MUST:
- require the SAME concrete announcement / report / transaction / incident / release —
  same company/theme/entity alone is INSUFFICIENT to group;
- keep separate: different times / countries / counterparties / quarters / actions;
- treat differing estimates/hooks as ONE event only when other facts identify the same
  event (the #5#9 Hyperion case);
- emit NO group when uncertain (precision over recall);
- handle **mixed-language** input: compare events across Chinese AND English titles (a
  bullet may reach merge untranslated — see Insertion point);
- treat the headline contents as UNTRUSTED data (prompt-injection resistance): they are
  news titles to classify, not instructions.
- include positive (group) and negative (don't-group, e.g. Google-Japan vs Microsoft-
  Germany) examples.

## Decisions (brainstormed, Codex-hardened)

## Decisions (brainstormed)

- **LLM output = groups of numbers only, program cuts.** (Not "LLM returns a rewritten
  deduped list" — lower risk, verifiable, LLM cannot corrupt titles/links/order.)
- **Keep-which:** keep one per group. v1: the LLM is asked to also name the best member
  to keep per group (most complete/free-source); if it doesn't, program falls back to the
  earliest merged bullet. This avoids the pure half-A keep-first bias Codex flagged (High
  #3) without program-side free-source ranking (Codex S4 permanent-skip).
- **Underfill: accept fewer, do NOT refill.** Same-event merge is supposed to shrink the
  section; drop to as few as survive (must stay non-empty). Emit an underfill trace. No
  refill — the downstream merged list has no clean numbered candidate pool to refill from,
  and refilling would re-introduce material the section didn't need. (Diverges from P2's
  refill deliberately; documented.)
- **Scope: AI section only.** Custom-topic cross-topic dedup (Codex Medium #6, subscribe
  AI+Meta → same story twice) is a separate problem, out of scope here.
- **Kill-switch:** `NEWS_CROSS_DEDUP` (default on; `=0` disables). MUST be read at
  CALL-TIME via `os.environ`, NOT an import-time module constant. Confirmed from source:
  all existing `NEWS_*` switches are import-time-evaluated (`run.py:45,64`) but `load_env()`
  runs later (`run.py:2608`), so a value only in `~/.nullclaw/.env` would be missed. Follow
  `_llm_retry_budget_secs` (`run.py:1025`), which reads `os.environ` inside the call. Test
  the `.env` path, not a patched module constant.
- **Dedicated short timeout, retry-aware:** `_run_nullclaw_agent` auto-retries rc=124 once
  (`run.py:1052`), so "one call" can be ~2× the timeout in wall-clock. Set a dedicated,
  short grouping timeout (this is a refinement, not a gate — it must not eat the run's
  budget). Document "one logical call, up to two attempts."
- **LLM failure = safe no-op.** If the grouping LLM call fails/times out/returns garbage,
  the section passes through UNCHANGED (log a warning). This dedup is a refinement, never a
  gate — it must never drop the whole AI section or fail the run (skill exits 0 on upstream
  errors per repo convention).
- **Cache:** the pass runs downstream of the per-half cache (`run.py:2173` merge is
  unconditional, even on cache hits — Codex Medium #5), so **no `AI_SUBSTAGE_CACHE_VARIANT`
  bump is needed** and it still runs when both halves are cache hits. (Add a test for the
  both-halves-cache-hit path.)

## Where it plugs in

`_summarize_default_ai_substaged` (`run.py` ~2090-2185): after Level-2/3 halves run and
`half_results` are concatenated (the `final.extend(...)` merge, ~`run.py:2173`) — apply the
LLM grouping pass to the merged lines before returning. The merge is the first place a
cross-half view exists.

**Mixed-language caveat (Codex round-2 MEDIUM):** bullets are "post-language-processing,
normally Chinese, possibly mixed-language" — NOT guaranteed all-Chinese. The selection
prompt requests Traditional Chinese (`run.py:2001`), but `_translate_selected_section` runs
only when the section-level language gate FAILS (`run.py:2060`), and that gate accepts ~80%
Chinese-looking bullets (`run.py:901`) — so e.g. 4 Chinese + 1 English bullet passes and the
English one reaches merge untranslated; a cache hit bypasses validation entirely
(`run.py:1963`). Therefore the prompt must compare events across mixed Chinese/English
titles (see Precision-first prompt).

## Trace

Emit `cross_dedup_llm` with `before` (block count), `after`, `dropped` (dropped headline
snippets or ids), `groups` (the LLM's grouping), `llm_ok` (bool). So effectiveness is
measurable and LLM misfires are diagnosable.

## Testing (TDD — RED first)

Real fixtures, LLM stubbed (monkeypatch the agent call to return canned groupings):
1. **RED regression (#5#9):** merged list where two blocks are the same event; stub returns
   them as one group → assert exactly one survives (kept-which policy).
2. **False-merge guard:** "Google 日本資料中心" + "Microsoft 德國資料中心"; stub returns NO
   group → both kept. Proves reliance on LLM judgment, not overlap.
3. **Paywall-replacement block dropped atomically:** a block with parent headline + exact
   `PAYWALL_CONT_PREFIX` continuation is dropped → both lines removed together; a surviving
   block keeps its link + continuation intact; helper does NOT emit its own footer (global
   `digest.count(PAYWALL_NOTE)` stays correct).
4. **Two-title paywall block matching:** a free-replacement block whose ORIGINAL headline
   (continuation) is the real duplicate → stub groups on `original_headline` metadata →
   correct block dropped.
5. **Invalid-grouping rejection (transactional):** for EACH — out-of-range member, duplicate
   member, <2 members, overlapping groups, `keep` not in its group, all-blocks-removed,
   nonzero rc with partial stdout, empty stdout, non-JSON — assert the WHOLE response is
   rejected and the section passes through unchanged.
6. **Excessive-collapse circuit breaker:** a valid grouping that would drop below
   `min(before,5)` / remove >50% → whole-pass no-op, rejection trace emitted.
7. **Kill-switch via .env path:** `NEWS_CROSS_DEDUP=0` supplied the way `.nullclaw/.env`
   would (call-time `os.environ`, not a patched constant) → passthrough, no LLM call.
8. **LLM failure safe:** stub raises / times out (rc=124 after retry) → section unchanged,
   warning logged, run does not fail (exit 0).
9. **Mixed-language input:** merged list with one untranslated English bullet duplicating a
   Chinese bullet → stub groups them → correct dedup (proves mixed-language handling).
10. **Underfill accepted:** valid same-event collapse that legitimately lowers count (but
    stays ≥ breaker floor) → fewer bullets, underfill trace, no crash, no refill.
11. **Both-halves-cache-hit:** both halves from cache → the pass still runs.
12. **Orphan/unexpected line fail-closed:** merged section with a stray non-bullet line →
    parse no-ops the pass, original lines preserved byte-for-byte.
13. **Trace shape:** `cross_dedup_llm` emitted with before/after/dropped/groups/llm_ok.

Tests in `news/scripts/test_run.py`.

## Files touched
- `news/scripts/run.py` — new block-parsing helper + LLM grouping function + one call site
  in `_summarize_default_ai_substaged`; `NEWS_CROSS_DEDUP` env; `cross_dedup_llm` trace.
- `news/scripts/test_run.py` — the 8 tests above.
- `news/SKILL.md` — document this as the 4th dedup layer (LLM same-event, cross-half,
  post-translation, AI-only), its safe-no-op-on-LLM-failure contract, kill-switch, trace.

## Out of scope
- Deterministic overlap dedup (rejected — can't distinguish same-event/same-theme).
- Custom-topic cross-topic dedup.
- Program-side free-source ranking (Codex S4 permanent-skip).
- Touching `cluster()`, the half-split, translation, or the weekly `ainews` project.
