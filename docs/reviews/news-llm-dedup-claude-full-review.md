# News LLM Dedup — Claude Full Code Review

日期：2026-07-13
Reviewer：Claude (Opus 4.8, 1M)
Mode：WRITE-ONLY review — no edits to `news/scripts/` or `lib/`.
Scope object：P0 `DEDUP_RULES` + P1 soft pair-hints + P2 post-select hard dedup + S3 underfill refill, across the three LLM entry points (default section, AI substage, custom topic).

---

## 1. Verdict

**Ship-ready with one documented pre-existing gap.** The dedup feature (P0 shared rules, P1 overlap≥4 soft hints, P2 greedy hard-dedup, S3 deterministic refill) is correctly implemented, well-tested (28 dedicated tests in `LlmDedupHintsTests`, 90/90 in the module), and matches both the Codex skip verdicts (`docs/reviews/news-llm-dedup-codex-skips-verdict.md`) and the SKILL.md description. The greedy-not-union-find invariant, first-in-LLM-order keep, comma-marker rejection, and no-revive-of-P2-dropped-ids are all enforced and directly tested.

The one substantive finding is a **minor, pre-existing** language-gate gap in the custom-topic path (§3, §4). It is *not introduced* by this dedup work — the custom-topic path never had a language gate — but the S3 refill now makes it marginally easier to surface an untranslated (English/Japanese) raw RSS title in a custom-topic digest. The default-section and AI-substage paths are **not** affected: their refill output is language-gated after post-dedup, so a refilled raw English title there is correctly routed to translation.

The prior-progress signal ("AI-substage refill can append raw RSS title past the language gate") was **investigated and does not reproduce for the AI substage** — see §4. It *does* apply to the custom-topic path.

---

## 2. Scope reviewed

Read in full:

- `news/scripts/run.py`:
  - `DEDUP_RULES` (L125–137), `TRANSLATION_RULES_STRICT` (L139–147)
  - `_dedup_pair_hints` (L531–559), `_format_dedup_hint_block` (L562–571), `_parse_pick_min` (L574–579)
  - `_post_dedup_selected_summary` (L582–713), `_refill_unselected_after_underfill` (L716–781)
  - `_topic_words` (L486–502), `_extract_leading_marker_ids` (L866–879), `_news_bullet_lines` (L835–850), `_neutralize_markdown_specials` (L820–832), `_MARKER_RE` (L1152)
  - `_language_validation_passed` / `_language_validation_stats` (L901–931), `_precheck_apply` (L1174–1272), `_attach_numbered_links` (L1284–1327), `_translate_selected_section` (L1548–1602)
  - Call sites: `_summarize_default_section` (L1787–1944), `_run_ai_substage` (L1947–2079), `_summarize_default_ai_substaged` (L2092–2182), `_run_custom_topic` (L2325–2457)
- `news/scripts/test_run.py`: `LlmDedupHintsTests` (L450–1131) + adjacent `ForbiddenEnglishAdverbTests`, spot-checks of others.
- `news/SKILL.md`: LLM 事件去重 section (L85–106).
- `docs/reviews/news-llm-dedup-codex-skips-verdict.md` (S1–S7).

Verification run:

```
$ python3 -m unittest test_run -v        → Ran 90 tests … OK
$ python3 -m unittest test_run.LlmDedupHintsTests -v → Ran 28 tests … OK
```

Both green. The "90 tests green" claim from prior progress is **confirmed**.

---

## 3. Findings by severity

### Blocker
None.

### Major
None.

### Minor

**M1 — Custom-topic path has no language gate; S3 refill can surface an untranslated raw RSS title.**
Evidence: `_run_custom_topic` (L2409–2457) runs `_post_dedup_selected_summary(..., pick_min=2)` (L2422) → `_precheck_apply` (L2433) → `_attach_numbered_links` (L2443) and returns. There is **no `_language_validation_passed` call anywhere** in the function. Contrast `_summarize_default_section` (language check at L1861) and `_run_ai_substage` (language check at L2060). When post-dedup underfills below `pick_min=2`, `_refill_unselected_after_underfill` (L769) appends `f"- #{cand} {safe}"` where `safe = _neutralize_markdown_specials(title)` — the **raw RSS headline**, which for many custom topics is English/Japanese. That line then passes marker validation (it carries `#N`), survives precheck (source not deny-listed), gets a link, and is delivered verbatim — untranslated.
- Severity rationale: minor, because (a) it is a **pre-existing** gap (the LLM's own custom-topic output already bypasses language validation — refill only widens an already-open door), and (b) it triggers only in the narrow window where the LLM selected ≥`pick_min` items, post-dedup collapsed them below `pick_min`, *and* a never-selected candidate with an English title happens to be the refill pick. But it is a real user-visible correctness gap: the whole point of `TRANSLATION_RULES_STRICT` + `_language_validation_passed` is that Traditional-Chinese-only reaches the user.
- Not a regression blocker for *this* change: the dedup feature's own two gated paths (default, AI) are safe. Recommend tracking as a custom-topic hardening follow-up (see §7).

### Nit

**N1 — SKILL.md L104 (S6) slightly overstates alignment.** It says "default / AI / custom 已對齊 P0+P1+P2". All three do share P0/P1/P2, but the custom path's *post-P2 handling* diverges: it lacks the language re-gate the other two have. Not wrong about dedup, but a reader could infer full parity of the surrounding validation pipeline. Consider a one-line note that custom-topic omits the language gate (by design or as known gap).

**N2 — `_refill_unselected_after_underfill` seeds `lines = [summary]` then re-joins (L750, L781).** For the default/AI callers `summary` is a multi-line block, so `"\n".join([summary, newbullet1, ...])` works, but it relies on `summary` never itself ending without a trailing newline needing care — it happens to be fine because each appended item is a full line. Purely stylistic; correctness is intact and tested (`test_post_dedup_underfill_trace_and_refill`).

---

## 4. Implementation correctness (P0 / P1 / P2 / refill / order / cache / env)

**P0 — shared `DEDUP_RULES` (L125–137).** Injected verbatim into all three prompts: default (L1835), AI substage (L2013), custom (L2398). Tests assert `run.DEDUP_RULES in prompt` for all three (L524, L560, L825, L867). The rule text correctly distinguishes 同事件 (collapse) from 同主題不同事件 (keep) and states free-source preference as *prompt text only* — matching S4's verdict that program-side source ranking stays out of keep decisions. **Correct.**

**P1 — soft pair hints (L531–571).** `_dedup_pair_hints` computes **independent** pairs (no transitive closure), sorts ids first for determinism, skips empty/missing titles (L549–555, tested L979–993), and emits only `overlap >= LLM_DEDUP_HINT_OVERLAP (=4)`. `_format_dedup_hint_block` returns `""` on empty and ends with `\n\n` (tested L486–491). Injected in all three paths behind `LLM_DEDUP_HINTS_ENABLED` with symmetric enabled/disabled `llm_dedup_hints` traces. The fixture test locks `pairs == [(1,5,7)]` and asserts no false hint for the 1∩2=3 theme pair (L460–471). **Correct**, and consistent with S2 (threshold stays ≥4, no entity lexicon).

**P2 — greedy hard dedup (L582–713).** Runs after marker validation, before precheck, in all three paths (default L1851, AI L2037, custom L2422). Algorithm is **greedy keep in LLM output order** (L648–656): a bullet is kept iff its overlap with *every already-kept* bullet is `< min_overlap`. This is explicitly **not** union-find:
- `test_post_dedup_no_transitive_bridge_drop` (L934–959) proves A–B and B–C edges don't drop C when A∩C<4. ✓
- `test_post_dedup_keeps_first_llm_order_not_lower_id` (L870–884) proves first-in-output wins over min-id. ✓
- `test_post_dedup_three_same_event_keeps_first_only` (L961–977). ✓
- `test_post_dedup_comma_marker_not_treated_as_id` (L995–1012) — `_MARKER_RE`'s `(?!,)` correctly rejects `#1,`. ✓
- Non-bullet lines preserved (L899–917). ✓
- `<2` selected → no-op with trace (L624–635, tested L886–897). ✓
Kill switch `NEWS_LLM_POST_DEDUP=0` returns summary unchanged with an `enabled:False` trace (L612–622, tested L680–695). **Correct.**

**Refill / S3 — `_refill_unselected_after_underfill` (L716–781).** Fires only when `pick_min is not None and len(selected) >= pick_min and 0 < len(after) < pick_min` (L690–694). Matches S3's "P2 後結果非空且低於 pick_min 時執行一次". Guarantees enforced:
- Does not re-call the LLM (pure loop). ✓
- Does not lower `min_overlap` (uses same 4). ✓
- Does not revive P2-dropped ids: `forbidden = set(llm_selected)` (L743) is the *full* LLM selection, so any id the LLM picked — kept or P2-dropped — is excluded (L755). ✓ (tested `test_refill_rejects_high_overlap_unselected`, L1045+, and the underfill test L1014–1043 shows only never-selected #3 is added.)
- Refill candidate must have `< min_overlap` with every already-kept item (L761–765). ✓
- Deterministic order: `for cand in sorted(numbered)` (L752). ✓
- Traces `post_dedup_underfill` (L695–703) and `post_dedup_refill` (L772–780) with attempted/added/final_count/still_underfill. ✓

**Order (the flagged concern).** Traced carefully per path:
- **Default section**: marker-valid (L1848) → `_post_dedup_selected_summary` incl. refill (L1851) → **`_language_validation_passed` (L1861)** → precheck → translate-or-attach. A refilled raw English title therefore hits the language gate and is routed to `_translate_selected_section` (L1882), which re-translates by marker id from `numbered`. **Safe.**
- **AI substage**: marker-valid (L2033) → `_post_dedup_selected_summary` incl. refill (L2037) → precheck (L2048) → `_resolve_paywall_replacements` (L2058) → **`_language_validation_passed` (L2060)** → translate-or-attach. Same protection: a refilled English title fails the gate and is translated (L2061). **Safe.** ⇒ The prior-progress hypothesis ("AI-substage refill appends raw RSS title past the language gate") **does not reproduce** — the language check at L2060 runs *after* the refill at L2037 on the same `summary`. `test_tech_language_fail_branch_still_post_dedups_before_precheck` (L1088) exercises the language-fail branch and confirms post-dedup precedes it.
- **Custom topic**: marker-valid (L2419) → `_post_dedup_selected_summary` incl. refill (L2422) → precheck (L2433) → attach (L2443). **No language gate** ⇒ finding M1.

So the refill/language ordering is **correct for the two paths that have a language gate**, and the residual exposure is confined to the custom-topic path, which lacks that gate entirely (independent of refill).

**Cache.** `AI_SUBSTAGE_CACHE_VARIANT = "default_ai_clustered_v5_post_dedup"` (L40) and custom variant `"custom_topic_v3_dedup"` (L2342) both embed the dedup semantics, so a variant bump invalidates same-day pre-dedup caches. `test_cache_variants_bumped_for_dedup` (L919–932) locks the AI variant string and asserts `custom_topic_v3_dedup` appears in `_run_custom_topic`'s source; `test_custom_topic_has_dedup_rules_hints_and_post_dedup` (L832) asserts a `custom_topic_v3_dedup` file is actually written. **Correct** — importantly, post-dedup + refill run *before* the cache write in all paths (AI L2078, custom L2453), so caches store the post-dedup body.

**Env / kill switches.** `LLM_DEDUP_HINTS_ENABLED` (`NEWS_LLM_DEDUP_HINTS`, L64), `LLM_POST_DEDUP_ENABLED` (`NEWS_LLM_POST_DEDUP`, L68), thresholds `LLM_DEDUP_HINT_OVERLAP == LLM_POST_DEDUP_OVERLAP == 4` (L70–71). Test asserts the equality invariant (L926). Both default-on when env unset (L928–929). **Correct.**

---

## 5. Test quality

Strong. 28 tests in `LlmDedupHintsTests`, all passing, covering:
- Word-overlap fixture calibration (`test_fixture_topic_word_overlaps`, locks 1∩5=7, 1∩2=3, 1∩3=2) — this is the backbone that makes every threshold assertion meaningful.
- Independence-not-transitivity for both hints and post-dedup.
- Greedy keep order, first-in-output, three-same-event collapse, comma-marker rejection, non-bullet preservation, single/empty no-op, kill switches, and the enabled/disabled trace shapes.
- All three call-site paths individually asserted to include `DEDUP_RULES` + hints + run post-dedup **before** precheck (via a `fake_precheck` that records the ids it sees — a clean way to prove ordering).
- S3 refill: happy path (adds never-selected), rejection of high-overlap unselected, underfill trace fields, and `_parse_pick_min`.

Gaps (not blockers):
- **No test for M1**: no test asserts what the custom-topic path does with a refilled *English* title. Every custom-topic test uses Chinese fixtures. A test feeding an English never-selected candidate into the refill window would document the current (untranslated) behavior — and would be the natural regression guard if M1 is fixed.
- **No direct unit test of `_refill_unselected_after_underfill` with `summary=""`** (the `lines = [summary] if summary.strip() else []` branch at L750). Covered indirectly since callers always pass a non-empty body, but the empty-body branch is untested.
- Refill tests exercise `_post_dedup_selected_summary(..., pick_min=…)` directly rather than through a call site, so the *interaction* of refill output with the downstream language gate (default/AI) is not asserted end-to-end. The ordering tests confirm post-dedup precedes the gate, but not that a specifically-English refilled bullet is translated. Low risk given §4's code trace, but worth a line.

---

## 6. SKILL.md vs code

Section L85–106 is accurate and current:
- P0/P1/P2 description (L95–97) matches implementation, including "greedy 保序 … **不做** union-find" and the refill rules ("不復活 P2 砍掉的 id、不二次 LLM"). ✓
- Threshold `overlap >= 4` stated for both P1 and P2 (L96–97) — matches constants. ✓
- Kill switches `NEWS_LLM_DEDUP_HINTS=0` / `NEWS_LLM_POST_DEDUP=0` named correctly. ✓
- Trace names `llm_dedup_hints` / `llm_post_dedup` / `post_dedup_underfill` / `post_dedup_refill` all match emitted events. ✓
- "套用：default tech/general、AI substage、custom-topic" for both P1 and P2 — matches all three call sites. ✓
- Codex skip verdicts (S1/S2/S4/S6, L99–104) faithfully summarize the verdict doc, and the doc-required S7 rewording ("**Codex 裁決為永久 skip**", not "刻意不做") is present at L99. ✓
- Cache variant string `default_ai_clustered_v5_post_dedup` (L106) matches `AI_SUBSTAGE_CACHE_VARIANT`. ✓

Discrepancies: only **N1** (S6 "已對齊 P0+P1+P2" is true for dedup but glosses the custom-path language-gate divergence). SKILL.md does not mention S3 refill by its "S3" label but describes the refill behavior fully in P2 (L97) — acceptable.

---

## 7. Recommended fixes (ordered)

1. **(M1, optional-but-recommended) Add a language gate to `_run_custom_topic`.** Mirror `_run_ai_substage` L2060–2071: after `_precheck_apply` + `_resolve_paywall_replacements`, if `not _language_validation_passed(summary)`, route through `_translate_selected_section(section_key, _extract_leading_marker_ids(...), numbered, date_str, paywall)` and cache/return the translated result; on `None`, return `False` so the caller falls back. This closes the custom-topic gap uniformly and makes S6's "已對齊" literally true across the whole pipeline. *(Out of scope for this write-only review; propose as follow-up.)*
2. **(N1) One-line SKILL.md note** that custom-topic currently omits the post-P2 language re-gate (state whether by-design or known-gap), so the parity claim isn't over-read.
3. **(Test, M1) Add a custom-topic test** with an English never-selected candidate that gets refilled, asserting the delivered bullet is Chinese (after fix #1) — or documenting the current untranslated behavior until then.
4. **(N2, cosmetic) Optional**: in `_refill_unselected_after_underfill`, build the output as `summary.splitlines() + new_lines` joined, rather than `[summary] + new_lines`, for clarity. No behavior change.

None of 1–4 block merging the dedup feature.

---

## 8. Ready to merge?

**Yes — merge the dedup feature (P0/P1/P2/refill) as-is.** It is correct, matches the design verdicts and SKILL.md, and is well covered by 90 green tests (28 dedup-specific). The refill-vs-language-gate ordering concern from prior progress does **not** reproduce on the default or AI-substage paths — the language gate runs after refill there. Track **M1** (custom-topic language-gate gap, pre-existing, mildly widened by refill) as a separate hardening follow-up per §7 item 1; it is a minor, narrow-window issue, not a regression introduced by this change.
