# chipcon `8d36b92` (Yahoo source switch) — code review note

**Date:** 2026-07-14
**Commit reviewed:** `8d36b92 feat(chipcon): fetch history from Yahoo, drop Stooq/price-CLI dependency`
**Method:** xhigh workflow review (21 agents, find → independent verify) + Claude line-by-line source corroboration. Both agreed on the core.

## Verdict: PASS — core fix correct, no blocker

The data-source switch is sound. Confirmed correct (review + independent check):
- Dead price-CLI code fully removed (`shutil`/`subprocess` imports, `price_cli_path`,
  `run_price_cli`, `parse_price_history_tsv`, `LOCAL_DEV_PRICE_CLI`) — no dangling refs.
- SMH-raise / secondary-degrade asymmetry correctly implemented (two-phase:
  per-ticker failures all degrade to `[]`+warning inside the loop, THEN
  `if not state.get("SMH"): raise` after — so only SMH hard-fails; QQQ/SOXX degrade + still deliver).
- `{logical: [[date, close]]}` state shape unchanged for classify/format_message.
- Sort correct: `sorted(rows, key=lambda r: r[0])` — `oil_fetch` emits dates via
  `date().isoformat()` (zero-padded `YYYY-MM-DD`), so lexicographic == chronological.
  DMA/trend order is safe. (Review verify pass REFUTED the sort-risk finding — agrees.)

## Non-blocking improvements (recorded, NOT yet fixed — deferred by user 2026-07-14)

Resilience / defensive:
1. **Hardcoded logical key `"SMH"`** (`run.py:242` guard + `classify`): the primary-position
   hard-fail is keyed on the literal `"SMH"`, not derived from `cfg["symbols"]`. If someone
   renames the primary logical key in config (e.g. `{"SEMI": "SMH"}`), a *successful* SMH
   fetch is misread as fatal → silent outage. Current/default config `{"SMH":"SMH",...}` does
   NOT trigger it — theoretical coupling fragility, not a live bug.
3. **`import oil_fetch` outside the try/except** (`run.py:13`): unlike `delivery`/`trace_marker`
   (which have fallback stubs), an `oil_fetch` ImportError crashes with a bare traceback BEFORE
   main()'s except → no `[skill-status:failed]` marker → breaks cron skill_contract. Low
   probability (oil_fetch is an existing lib), but real.

Behavior / diagnostics:
2. Record mode: SMH fetch failure now raises → writes NO history-log line (old code wrote an
   INSUFFICIENT_HISTORY row) → silent gap on SMH-outage days.
4. SMH-failure RuntimeError message concatenates secondary (QQQ/SOXX) warnings too → blended
   root-cause noise for the operator.
5. Secondary-ticker `except Exception` swallows data corruption (`float()` failure) into a
   soft warning (PLAUSIBLE).

Test quality (this commit's rewritten tests):
6. **No test asserts `range_name="1y"` is forwarded** to `fetch_history` (fakes use `**kwargs`).
   A future edit to `5d`/dropped arg passes all tests but silently produces INSUFFICIENT_HISTORY
   in prod. — highest-value test gap.
7. Success test's fake is symbol-agnostic → a symbol→key mismapping would pass undetected.
9. `test_run.py` uses pytest, contra repo CLAUDE.md "plain unittest, no pytest" (pre-existing,
   not introduced by this commit).

Docs:
8. `emit()` docstring still references "Stooq" (removed by this commit; only SKILL.md was updated).

## Refuted by the review's own verify pass (agreed, not real)
sort-risk (isoformat safe), future lib-signature change (not a current defect), redundant sort,
unrealistic UNSORTED test input.
