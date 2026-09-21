# Phase ③ Plan 2 (inflation-con) — intentional differences from the Python

Every entry is a deliberate decision. Anything not listed here is a bug.
Python oracle: `inflation-con/scripts/run.py` (unchanged at cutover).
Rust crate: `crates/inflation-con`.

Acceptance is **parity with the Python on identical inputs**, not
`status=ok`. The skill fires on days 3–5 of each month (UTC+8), so a soak
that only watches a week cannot see a scheduled run. Keep the Python entry
point available for a full monthly cycle after the `## Script` switch.

## Dropped / structural

1. **Unreachable YELLOW dead code is not ported.** In Python, the YELLOW
   branch contains `if not core_cpi_hot_3_or_6: reasons.append("core CPI not
   confirming yet")` (`run.py` ~187–188). YELLOW's entry condition already
   requires `core_cpi_hot_3_or_6` to be true, so the negation never holds and
   that string has never printed. Carrying it across a translation would leave
   the next reader hunting for a case that cannot fire. Recorded here so its
   absence is not mistaken for a missing branch.

2. **`format_message`'s `cfg` parameter is unused on both sides.** Python
   accepts `cfg: dict` (`run.py:253`) and never reads it in the body. Rust
   keeps the parameter as `_cfg: &Config` for signature parity with the
   call site; it does not influence the rendered message or skill status.

3. **`load_config` keeps only `series` and `policy_stance`.** Python's
   `load_config` returns the full JSON dict after rewriting those two keys,
   so unknown top-level keys remain in the returned mapping. Rust's `Config`
   accepts only the two fields the skill uses. No skill behaviour depends on
   the extras (they were never read after load).

4. **`record_line` takes the clock as a parameter.** Python stamps
   `datetime.now(CST)` inside the function (`run.py:287–288`). Rust takes
   `now: &str` so the history-line shape is unit-testable with a pinned
   clock (same pattern as chipcon Plan 1). Production `main` passes the real
   CST stamp; observable output on a live run is the same shape.

## Fetch / FRED transport

5. **Fixed FRED User-Agent, asserted in a test.** The live transport sends
   `User-Agent: curl/8.5.0 nullclaw/1.0` (`fred_fetch.py:32`; Rust
   `fetch::USER_AGENT`). This is **not** inherited from market-fetch's
   default: the constant and the header table are asserted in
   `tests/fetch.rs` so a regression that drops back to a bare
   `nullclaw/1.0` turns red. A bare `nullclaw/1.0` does not return 4xx —
   FRED **hangs the connection**, so the symptom is a timeout that looks like
   a network fault rather than an auth/UA rejection. Measured when the
   Python was corrected after three weeks of silent breakage.

6. **`CreditError::NoData` maps to an empty series.** Python's
   `fred_fetch.parse_csv` returns `[]` for a header-only CSV, an all-`.`
   CSV, and an empty body. market-fetch's `parse_fred_csv` returns
   `Err(CreditError::NoData)` for all three. Without the mapping, the same
   upstream response yields `fetch {id}: no usable observations` instead of
   `{id}: no rows` — that text reaches the delivered message and the history
   line, and the Task 5 differential compares it character for character.
   An earlier draft of the plan said no such mapping was needed (reasoning:
   `fetch_all` already tolerates each series, so a throwing adapter cannot
   crash the run). That was wrong: the warning **wording** is the contract,
   not only the control flow. `Http` and `Parse` stay errors.

7. **`cosd` is always sent; its inertness is a measurement, not an
   identity.** `market-fetch::fred::build_url` always includes `cosd`
   (Rust uses `1900-01-01`); the Python omits it. Measured 2026-07-29 across
   all seven configured series: **identical row counts with and without
   `cosd`** (PCEPILFE 809, CPILFESL 834, PCEPI 809, CPIAUCSL 954, T10YIE
   6149, DFII10 6148, DGS10 16845). That is an observation about FRED's
   **current** default window on that date, not a mathematical identity. If
   FRED tightens the default window later, `core_pce_obs` (which is
   rendered) can change while the Python stays on the open default.

## Config order

8. **Series order follows `DEFAULT_SERIES`, then extra keys in file
   order.** Python does `dict(DEFAULT_SERIES)` then `.update(file)`:
   existing keys keep their DEFAULT position; new keys append in file
   insertion order. Rust mirrors that merge. Extra-key order depends on
   `serde_json`'s `preserve_order` feature — without it, file extras can
   reorder and the warning join in `fetch_all` (config order, not
   alphabetical) drifts. A test that only shuffles the seven defaults pins
   nothing; the load-bearing test uses extra keys.

## Operational (not fixed in this phase)

9. **A `degraded` run still delivers and still trips `retry_once`.**
   nullclaw's `cron.zig` retries when `verified != 1`. Classification:

   | skill-status | verified |
   |---|---|
   | `ok` | 1 |
   | `degraded` | 2 |
   | `failed` | 3 |

   So a successful delivery that then emits `degraded` (secondary series
   warn, core PCE fine) is retried with the same `NULLCLAW_JOB_ID`, and the
   skill cannot tell it is the retry. Result: **two Telegram messages** for
   one logical run. Same gap weather's Option A hard-failure path left open
   for its own degraded path. Not addressed in Phase ③ Plan 2; recorded so
   a double delivery after cutover is not mistaken for a Rust regression.

10. **`load_config` runs outside `main`'s try** — preserved wart. A
    malformed config produces neither markers nor the controlled
    `INFLATION-CON failed:` line. Same as chipcon and the Python.

## Differential and preflight (Task 5 / Task 6)

- Task 5 fixture differential: byte-identical to Python on captured FRED
  CSVs for the WATCH (live-shaped) path, a synthesised hot RED set, and the
  YELLOW boundary-note case (levels reach RED, context fails).
- Task 6 live preflight (2026-07-30, `--deliver-to` omitted): Rust and
  Python rendered messages were byte-identical (both `WATCH`, same indicators
  and reasons; stdout sha256
  `d075fbd2ff79fa79433ac4ed51b46a89ebca0d4c6737248a412f69d2e37d4804`).

## Cutover acceptance and rollback

**Accept** after one scheduled run (days 3–5) whose skill-status matches the
Python's for the same inputs and whose history-line shape matches the
goldens; keep the Python entry point available for a full monthly cycle.
Do not treat a week of quiet as soak evidence — the cron does not fire daily.

**Rollback triggers** (revert the `## Script` line to
`~/.nullclaw/skills/inflation-con/scripts/run.py`; the Python entry point
stays in place throughout):

- classification / rendered message differs from the Python on identical inputs;
- a Rust-only non-zero exit;
- marker text, ordering, or exit code differing from the contract goldens;
- a history-line shape that changed relative to the Python.

**Deployment identity:** the deployed binary's sha256 must match the
`cargo build --release -p inflation-con` artifact recorded at cutover. A
green rebuild without reinstall leaves the old binary running (observed
2026-07-30 for weather and doughcon).

## Build artifact at Task 6 preflight (2026-07-30)

```
481f5233c6557f53b5713f2e160aed7a43362feb8f451917062b801ca1ee16eb  target/release/inflation-con
```

Install and `## Script` switch are human-authorised steps; see the Task 6
report for the prepared commands (not executed by the agent).
