# Phase ③ Plan 3 (oilcon) — intentional differences from the Python

Every entry is a deliberate decision. Anything not listed here is a bug.
Python oracle: `oilcon/scripts/run.py` (unchanged at cutover).
Rust crate: `crates/oilcon`. Cut over 2026-07-31; first scheduled Rust run
2026-07-31 22:00 Taipei.

Acceptance is **parity with the Python on identical inputs**, not `status=ok`.
The distinction matters more here than in the other two ports, because in
production the two implementations **do not have identical inputs** — see
"The historical base changed" below, which is the most consequential entry.

## The historical base changed, and it changes every derived number

The Python's store (`oilcon-yanggf8`) and the Rust's (`price-registry`) hold
**different closes for the same symbol on the same date**:

| | `oilcon-yanggf8` | `price-registry` |
|---|---|---|
| `CL=F` 2026-07-28 | 81.41999816894531 | 79.26000213623047 |
| `CL=F` 2026-07-29 | 84.72000122070312 | 84.45999908447266 |
| `HO=F` 2026-07-29 | 4.17110013961792 | 4.370100021362305 |

Not a row-count difference. Verified against the Yahoo 1-year payload captured
for the Task 6 differential: its closes for those days are **79.26 and 84.46,
matching `price-registry` exactly**. The Python accumulated its history one row
per night, each row being whatever `fetch_latest` returned at **22:00 Taipei** —
mid-session for the US market. The Rust's history came from a single backfill of
Yahoo's **settled daily closes**. Both stamp the same date.

Consequences:

- **Measured on 2026-07-30, same day, same clock**: `ma50` 81.78 vs 82.67,
  WTI change −1.2% vs −0.9%, Brent −0.4% vs −1.3%, HO −1.8% vs −6.3%, distance
  below the 60-day high 19.4% vs 23.0%. The current close, both one-year extremes
  with their dates and day-counts, and the classification all agreed.
- **It can change the classification.** `classify_oil_trend` branches on
  `pct_below <= 10.0`; a 3.6-point gap in that figure puts the two on opposite
  sides of the boundary on a day sitting near it. This is not display-only.
- Going forward both append an intraday snapshot for the current day, so the
  divergence is in the **year-long base**, not in today's row.
- The Rust series is arguably the better one — a settled close is what a daily
  series should hold, and a 22:00-Taipei snapshot is an artefact of when the job
  runs — but that is a preference, not a licence to leave it undeclared.
- **Rollback is exact**: `oilcon-yanggf8` is untouched, so reverting the
  `## Script` line restores the old numbers as they were.

## Storage

- **The store moved** from oilcon's own `oilcon-yanggf8` (`oil_daily`) to the
  shared `price-registry` (`prices`), read through `price-store`.
- **The backfill guard replaced a presence check.** `oil_store.needs_backfill` is
  `SELECT 1 … LIMIT 1`. Safe when oilcon owned its database; against a shared
  table one row from any writer would suppress the year-long backfill forever.
  Replaced by coverage: `rows < 70` **or** newest older than `today − 7` **or**
  `span_days < 300`, all three needed.
- **Reads are Yahoo-only, filtered in SQL** via
  `price_store::read_window_from_source(conn, ticker, "yahoo", 252)`. Foreign
  rows are invisible to oilcon, not repaired. A repair design was rejected: it
  can never converge, because `upsert_many` is an UPSERT and a foreign row on a
  date Yahoo does not return survives and re-triggers the repair on every run.
  The only convergent variant has oilcon deleting another writer's rows from a
  shared table.
- **Known limit, pinned by a test:** 70 rows spread over a 300-day span satisfies
  all three conditions and does **not** backfill, which is thin sampling for
  extremes the message calls a year. No density condition was added — this is the
  port's only logic without a Python oracle, and the shape can only arise from a
  partially failed write.
- **Assumption, recorded rather than left as coincidence:** `MIN_SPAN_DAYS = 300`
  against `WINDOW_SIZE = 252` is satisfiable only because Yahoo returns trading
  days. 252 calendar-dense rows span 251 and the guard would fire forever. Measured
  at the preflight: 252 trading days really do span 365 calendar days.

## Failure handling

- **A history-fetch failure on a populated store no longer aborts.** The Python
  re-raises unconditionally, discarding symbols already built. The Rust resolves
  through `after_failed_refresh`: rows present → keep them, mark the symbol stale,
  continue; nothing stored → abort as the Python does. A 30-row symbol therefore
  survives a failed refresh, clears `MIN_HISTORY_ROWS`, and classifies as
  `insufficient-history` — degraded rather than a lost report.
- **`Upstream` and `NoData` both map to an empty series**, matching the Python's
  `parse_chart_response`, which returns `[]` for `chart.error`, a missing
  `result`, or falsy closes. A consequence given up deliberately: **a delisted or
  renamed oil symbol surfaces only indirectly** — as `n/a`, stale, or insufficient
  history. That is what happens today.
- **The `[WARN: turso unavailable - …]` text differs.** Python interpolates
  `str(exc)` from its driver; the Rust produces `kind: message` from
  `turso_util::Error`, which implements neither `Display` nor `std::error::Error`.
  Not byte-comparable across drivers. `Debug` was deliberately not used — it would
  have shipped `Error { kind: Turso, message: "…" }` to a Telegram reader.
- **The history-log error line** interpolates a Rust `io::Error` where Python
  interpolates an `OSError`; the wording differs by language, the shape does not.

## Argument parsing

- **`argparse`'s refusals are reproduced, including the exit code.** Unknown flag,
  invalid `--mode` value, and a flag missing its value all exit **2**, before
  `build_snapshot`, so a bad argument never reaches a fetch or a write.
- The original port silently ignored unknown flags and accepted any `--mode`
  string. Not cosmetic: `if args.mode == "deliver"` is false for a typo, so
  `--mode recrod` fell through to the **record** branch, which never delivers —
  the nightly signal would have stopped while the run still emitted
  `[skill-status:ok]`. Caught by `tools/install-skill.sh`'s smoke probe, which a
  manual `install` had bypassed.
- **The message text is not byte-comparable** with `argparse`'s usage block and is
  not attempted. The exit code is the contract.

## Structural

- **`format_message` and `format_record_line` take the clock as a parameter.**
  The Python calls `cst_now()` inside both (`run.py:202`, `:237`). Injected here
  for testability and for the differential, which substitutes
  `oilcon_run.cst_now` on the module object rather than editing the file.
- **The window is read after the latest upsert**, matching the Python's single
  `window()` call, which sits below it. An earlier plan revision put the read
  before `fetch_latest`; that would have left today's close committed to the store
  but absent from `rows`, making every rendered number one observation stale.
- **`compute_extremes` returns the first of a tied high.** Python's `max()` takes
  the first maximum; Rust's `Iterator::max_by` takes the last. `min` agrees in
  both languages, which is what makes the asymmetry easy to miss. `high_day` and
  `days_since_high` are rendered, and ties between daily closes are ordinary.
- **The `OIL-TREND` display comparator disagrees with the classifier's, and that
  is preserved.** The line renders `above`/`below` on `>=` while
  `classify_oil_trend` branches on `>`, so at exactly equal price it reads
  `rollover (… above 50MA …)`. Ported as-is: it is live rendered output, and
  tidying it in passing would be a silent behaviour change.
- **`Snapshot` is three fields rather than a dict**; the all-or-nothing abort
  clears three `rows: None` instead of emptying a map. Same meaning.

## Operational (not fixed in this phase)

- **A `degraded` run still delivers and still trips `retry_once`, so it can
  deliver twice.** `emit_and_exit` delivers before emitting markers; the scheduler
  classifies `degraded` as `verified = 2` and `repair_policy = retry_once` re-runs
  it. **This is not theoretical**: `2026-07-20 22:00:05` recorded
  `status=error`, `verified=2`, `failure_class=contract_degraded`,
  `repair_action=retried_failed` — two deliveries, under the Python. Preserved
  deliberately. **A degraded run is therefore not a rollback trigger.**
- **`--mode record` is not scheduled.** There is one oilcon cron job and it
  delivers. Record mode is verified by explicit manual invocation; no scheduled
  run will ever produce a history line.
- **Credentials resolve from the environment, not the token cache.**
  `turso_util::resolve_token` checks `PRICE_TURSO_WRITE_TOKEN` first and returns
  immediately; the gateway environment carries it and it has no `exp` claim. The
  1-day cache and the `turso` CLI are only the fallback for manual runs.

## Differential and preflight

- **Task 6 differential**: six fixture sets — live Yahoo plus four synthesised
  trend states plus the fixed-point equality case — **byte-identical** on message,
  record line and skill status. `tests/differential.rs` spawns `python3` at test
  time; nothing is compared against a stored expectation. No network: zero
  `connect()` and zero `AF_INET` sockets under `strace` on both sides.
- **What the differential does not cover**: every intentional divergence above
  stayed untriggered, because all sets are pre-seeded with successful fetches.
  Those paths are covered by unit and contract tests instead.
- **Preflight (2026-07-30)**: `--mode record` — which already was the
  "write the registry, deliver nowhere" mode, so the cutover needed no new code.
  Exit 0 in **5.1 s** against the job's 120-second timeout. All three tickers
  reached 252 rows spanning 365 days, source `yahoo` only, clearing every backfill
  condition. Every other ticker in the shared table was byte-identical to the
  pre-run baseline.

## Cutover and rollback

- Binary published by `tools/install-skill.sh oilcon`, `sha256`
  `18362db323145b503170ef206ae7f44eda7eb755963a9267c8eaf516d22e6f5d`.
- **Rollback**: set `## Script` back to
  `~/.nullclaw/skills/oilcon/scripts/run.py`. `oilcon-yanggf8`,
  `TURSO_DATABASE_URL` and `TURSO_AUTH_TOKEN` are untouched throughout.
- **Rollback triggers**: a Rust-only non-zero exit; marker text, ordering or exit
  code differing from the goldens; stored coverage below the backfill guard after
  a run that reported success; or a credential that cannot be renewed.
  **Not** a rollback trigger: a `degraded` run, or figures differing from the
  Python's — both are expected and explained above.
- **Out of scope**: retiring `oilcon-yanggf8`. It is the only rollback dataset,
  since history is not migrated. It stays, with its token, until acceptance is met.
