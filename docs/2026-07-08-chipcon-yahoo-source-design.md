# chipcon: switch price source Stooq → Yahoo, remove price-CLI dead code

**Date:** 2026-07-08
**Status:** design corroborated (Codex, GO-WITH-FIXES — 3 fixes folded in; Yahoo re-probed live with oil_fetch's exact UA: SMH/QQQ/SOXX 251 rows each) — ready for plan

## Problem

chipcon gets prices via the shared `price` CLI, whose only source is Stooq CSV
(`~/b/gwebcdb/crates/price-cli/src/source.rs`). Stooq is currently serving its
anti-scraping page for SMH/QQQ/SOXX, so `price fetch` returns garbage and only
~5 rows accumulate in the registry → chipcon is stuck at `INSUFFICIENT_HISTORY`
(needs 20). Two coupled weaknesses: (a) Stooq blocks scraping, (b) Stooq gives
only the latest close, so chipcon must slowly accumulate 20 daily rows before
trend can be computed.

## Decision

Switch chipcon to fetch history directly from Yahoo Finance, using the existing
shared helper `lib/oil_fetch.py` (`fetch_history(symbol, range_name="1y")` →
`[(date, close), ...]`, JSON `query1.finance.yahoo.com/v8/finance/chart`, no key,
UA `nullclaw/1.0`). Yahoo is already used live by `cct2` for stock quotes.

Verified 2026-07-08 (direct probe): SMH/QQQ/SOXX each return **251 daily rows**
(2025-07-08..2026-07-07) — far above the 20-row threshold, so
`INSUFFICIENT_HISTORY` disappears and trend is computable immediately.

**Scope: option A (skill layer).** Change only `chipcon/scripts/run.py`
`update_state`; leave the `price` CLI and its Rust source untouched (other
skills still use it). Do NOT touch `~/b/gwebcdb`.

## Change

`update_state(cfg)` — same return contract `(state, warning)` where
`state = {logical_key: [[date_str, close_float], ...]}` ascending, consumed
unchanged by `classify`/`format_message`:

```python
import oil_fetch  # SKILLS_LIB is already on sys.path
def update_state(cfg):
    symbols = cfg.get("symbols", {"SMH": "SMH", "QQQ": "QQQ", "SOXX": "SOXX"})
    warnings, state = [], {}
    for key, symbol in symbols.items():
        sym = str(symbol).upper()
        try:
            rows = oil_fetch.fetch_history(sym, range_name="1y")
            # FIX 2: sort by date ascending — classify assumes chronological
            # order (rows[-1] is "current"); oil_fetch preserves Yahoo payload
            # order and does not guarantee it.
            rows = sorted(rows, key=lambda r: r[0])
            if not rows:
                warnings.append(f"yahoo {sym}: no rows")
            state[str(key)] = [[d, float(c)] for d, c in rows]
        except Exception as exc:
            warnings.append(f"yahoo fetch {sym}: {exc}")
            state[str(key)] = []

    # FIX 1: hard-fail semantics. The current price path raises on non-partial
    # failures and main() turns that into [skill-status:failed] + exit 1. SMH is
    # THE monitored position — no SMH data means no signal, so raise rather than
    # deliver a hollow degraded INSUFFICIENT_HISTORY. A missing secondary ticker
    # (QQQ/SOXX) stays a partial degrade (relative-strength fields go None,
    # which classify already tolerates).
    if not state.get("SMH"):
        raise RuntimeError(
            "; ".join(warnings) or "yahoo: no SMH history (primary symbol)"
        )
    return state, "; ".join(warnings) if warnings else None
```

Semantics (FIX 1): SMH fetch failure / empty → **raise** → `[skill-status:failed]`
exit 1 (matches the old non-partial hard-fail). A secondary ticker (QQQ/SOXX)
missing → warning + empty list + still delivered `degraded` (classify tolerates
None relative-strength). This intentionally preserves the current hard-vs-partial
split rather than silently downgrading a total outage.

## Dead code to remove (verified: used ONLY by the price-CLI path)

`run.py`: `LOCAL_DEV_PRICE_CLI`, `price_cli_path`, `run_price_cli`,
`parse_price_history_tsv`, and the now-unused imports `shutil` and `subprocess`.
(`grep` confirmed `shutil.`/`subprocess.` appear only inside those two
functions.)

`test_run.py` (FIX 3 — full inventory): remove tests exercising the removed
path — `price_cli_path` resolution (env/config/PATH, test_run.py:64-74),
`parse_price_history_tsv` (test_run.py:55), and the **three**
`run_price_cli`-monkeypatched `test_update_state_*` tests (test_run.py:77, 102,
115). Removing the price path also makes the `cp()` helper (test_run.py:50) and
the test-file `import subprocess` (test_run.py:2) dead — remove both. Replace
with `oil_fetch.fetch_history`-monkeypatched `update_state` tests:
- success (all 3 tickers, unsorted input) → state sorted ascending, correct
  shape `[[date, float]]`, no warning;
- SMH fetch raises / SMH empty → `update_state` **raises** RuntimeError (FIX 1);
- secondary ticker (QQQ) raises → warning recorded, QQQ empty list, SMH present,
  returns degraded (no raise);
- empty rows for a secondary ticker → warning.

## Config / network

- `config.symbols` maps logical→Yahoo ticker 1:1 (SMH→SMH …); Yahoo accepts
  these tickers verbatim (probed).
- `CHIPCON_PRICE_CLI` env override and `price_cli_path` config key are removed
  (no longer meaningful).
- Network dependency moves from Stooq to Yahoo; **SKILL.md must update** the
  Data Store section (SKILL.md:13 intro, SKILL.md:62 "owned by price / Stooq CSV
  / CLI resolution order") to describe direct Yahoo `fetch_history` via
  `lib/oil_fetch.py`, and drop the `CHIPCON_PRICE_CLI` / `price_cli_path`
  mentions. `range_name="1y"` (251 rows) is ample for 20/50-DMA.

## Testing / rollout

- `pytest test_run.py` green in source-of-truth; deploy the copy to
  `~/.nullclaw/skills/chipcon`.
- End-to-end: run chipcon live (real Yahoo, real Telegram) → expect a real
  status (OK/YELLOW/…) and `[skill-status:ok]`, no `INSUFFICIENT_HISTORY`.
- Pipeline: design → Codex corroborate → plan → Grok corroborate → Grok writes
  (RED where behavior is new) / Claude audits every diff / Claude reverifies.

## Out of scope

- Removing Stooq from the `price` CLI itself (that is option B, a Rust change in
  gwebcdb) — deferred; other skills keep using `price`.
- Multi-source/fallback provider abstraction — YAGNI for chipcon.
