# oilcon — Daily Oil Index Skill (Design)

**Status:** Draft for review
**Date:** 2026-04-15
**Author:** yanggf
**Repo:** `~/a/claw-skills/` (nullclaw skill, same pattern as `doughcon`, `stock`)

---

## 1. Purpose

A daily-delivered Telegram digest of oil prices, watched passively by the user to spot a regime turn — the moment WTI bottoms and begins climbing off the low. Historically, this turn precedes airline-ETF (JETS) recoveries. The skill reports; the user decides.

**Explicitly not:**

- A trading system or entry/exit signal generator
- A state machine (BULLISH/NEUTRAL/BEARISH)
- An alerting system with thresholds, cooldowns, or spike detection
- A classifier of any kind

The skill shows numbers. The human reads them every morning the same way they read `doughcon` and makes their own call.

## 2. User Story

The user currently holds no JETS position and is waiting for an entry. Their mental model: oil is elevated (~$100), will grind lower over weeks/months, eventually bottom somewhere (maybe $75–80), and then turn back up. The turn off the bottom is the airline-ETF buy signal. They want a daily Telegram message that, read over time, lets them see the grind-down and catch the turn-up as it develops.

A sudden large jump in oil (e.g. 35% in a short window) is the opposite regime — a supply shock — and the user will see it immediately in the daily message's "today's change" line without needing a separate alert path.

## 3. Signal Design

### 3.1 Instruments

| Role | Symbol | Why |
|---|---|---|
| **Headline / signal** | `CL=F` (WTI) | Deepest futures liquidity, fastest price discovery, the number the user's intuition is calibrated to |
| **Confirmation** | `BZ=F` (Brent) | Global benchmark; agrees with WTI ~0.95 of the time; disagreement flags a WTI-specific quirk |
| **Confirmation** | `HO=F` (Heating oil) | Distillate proxy for jet fuel; tracks what airlines actually burn; disagreement flags refining-margin divergence |

**Design rationale:** WTI leads. Brent and HO follow within a day. For daily-frequency turn detection, WTI alone is sufficient signal — Brent and HO exist only to answer "is WTI telling the truth?"

### 3.2 Derived Values (WTI only)

Computed from the most recent **up to 252 trading days** of stored daily closes (the "window"). The window is whatever rows exist for the symbol with the 252 most recent dates — gaps are not repaired, and fewer rows are acceptable above the minimum (see §5.5). Values below:

- **Current price** — latest close
- **Today's change** — `(close_today - close_prev) / close_prev`, as signed percent
- **Recent high** — max close in the 252-day window, with its date
- **Days since high** — trading-day count from high-date to today
- **Distance from high** — `(close_today - high) / high`, signed percent (negative means below high)
- **Recent low** — min close in the 252-day window, with its date
- **Days since low** — trading-day count from low-date to today
- **Distance off low** — `(close_today - low) / low`, signed percent (positive means above low)

Lookback is a **rolling window of the 252 most recent stored trading-day rows** (ORDER BY date DESC LIMIT 252). Hardcoded constant, no configuration surface. The window is a row count, not a calendar window — 252 trading days is "roughly one year" but the skill never reasons about calendar dates for the lookback. High/low dates displayed in the message come from the row metadata.

### 3.3 Confirmation Logic (Brent, HO)

For each confirmation symbol:

- `✓` if `sign(today's change) == sign(WTI's today's change)`
- `✗` otherwise

That is the entire logic. No magnitude threshold, no weighting. Deliberately dumb so the user can trust it at a glance.

### 3.4 What Is NOT Computed

Explicitly excluded from this design: moving averages, crossovers, momentum, volatility, z-scores, state classifications, JETS correlation, VIX regime filters, alert transitions, spike detection, cooldowns, state files. All considered and rejected. They belong to a different project.

## 4. Output Format

Telegram message, UTF-8, ~10 lines:

```
🛢️ OILCON 情報

WTI: $78.20 (+0.9%)
  高 $92.40 (2026-01-08, 97日前, -15.4%)
  低 $74.10 (2026-03-02, 44日前, +5.5% off low)

確認：Brent ✓ (+0.7%)   HO ✓ (+0.8%)

更新：2026-04-15 17:00
```

Traditional Chinese (Taiwan) labels, per user's global preference. All prices USD. Dates in `YYYY-MM-DD`.

**Scheduler markers are NOT part of the Telegram body.** `[skill-status:*]` and `[trace:<JOB_ID>]` are printed to local stdout *after* `delivery.deliver_or_fail()` returns, via `lib/trace_marker.py`. They never reach the user's chat. See `doughcon/scripts/run.py` for the canonical sequence.

**Degraded output** (any fetch failure, turso unavailable, or insufficient history — see §7 for the full matrix):

```
🛢️ OILCON 情報
[WARN: <specific reason>]
更新：2026-04-15 17:00
```

The degraded body is passed to `deliver_or_fail()` the same way as a healthy body. After delivery, the script emits `[skill-status:degraded]` to stdout, then `[trace:<JOB_ID>]`. The skill process exit code in deliver mode is governed entirely by `deliver_or_fail()`'s contract (see §7) — **not** by whether the body is healthy or degraded. A degraded body that delivers successfully exits 0; a healthy body whose Telegram send fails exits 1 via `sys.exit` inside `deliver_or_fail`.

## 5. Architecture

### 5.1 File Layout

```
~/a/claw-skills/
├── lib/
│   ├── delivery.py         (existing)
│   ├── telegram.py         (existing)
│   ├── trace_marker.py     (existing)
│   ├── oil_fetch.py        NEW — Yahoo chart API client for oil symbols
│   └── oil_store.py        NEW — turso client: schema, backfill, insert, query
└── oilcon/
    ├── SKILL.md            NEW
    └── scripts/
        └── run.py          NEW — CLI, formatting, orchestration
```

**Rationale for splitting into `lib/`:** `oil_fetch.py` and `oil_store.py` are the genuinely reusable pieces — any future commodity-watching skill (copper, gas, gold) will need a Yahoo daily-close fetcher and a time-series store. Keeping them in `lib/` honors the user's preference for incremental extraction without inventing a framework. `run.py` stays oilcon-specific.

### 5.2 Data Flow (daily run)

```
cron → run.py
  → oil_store.ensure_schema()        (idempotent CREATE TABLE IF NOT EXISTS)
  → oil_store.needs_backfill(symbol) (true on first run, false after)
    ↓ if true
  → oil_fetch.fetch_history(symbol, 252)   (Yahoo, one call per symbol)
  → oil_store.insert_many(rows)
    ↓
  → oil_fetch.fetch_latest(symbol)   (Yahoo, latest close)
  → oil_store.upsert(symbol, date, close)
  → oil_store.window(symbol, 252)    (returns list of (date, close))
  → compute derived values (WTI)
  → compute confirmation flags (BZ, HO)
  → format message
  → delivery.deliver_or_fail(message, chat_id)
```

### 5.3 Turso Schema

```sql
CREATE TABLE IF NOT EXISTS oil_daily (
  symbol TEXT NOT NULL,
  date   TEXT NOT NULL,           -- YYYY-MM-DD, trading-day date
  close  REAL NOT NULL,
  PRIMARY KEY (symbol, date)
);
CREATE INDEX IF NOT EXISTS idx_oil_daily_symbol_date
  ON oil_daily(symbol, date DESC);
```

One table, three symbols (`CL=F`, `BZ=F`, `HO=F`). Close price only — no OHLC, no volume. Scales to a fourth instrument without schema change.

**Connection:** `libsql-experimental` Python client (stdlib-only isn't an option here; this is the first skill with a dependency). Credentials from environment:

- `TURSO_DATABASE_URL` — placeholder, user fills in
- `TURSO_AUTH_TOKEN` — placeholder, user fills in

Both read via `os.environ.get(...)`. If either is missing, skill emits `[WARN: turso credentials missing]` and exits degraded. Add dependency to skill's `requirements.txt` (new file, oilcon-local).

### 5.4 Yahoo Fetch

Endpoint: `https://query1.finance.yahoo.com/v8/finance/chart/<symbol>?interval=1d&range=1y`

URL-encode the `=` in symbols (`CL%3DF`). User-Agent header `nullclaw/1.0`, matching `stock/run.py`. Parse `chart.result[0].timestamp[]` and `chart.result[0].indicators.quote[0].close[]` into `(YYYY-MM-DD, close)` pairs. Skip any null closes (Yahoo occasionally emits them for half-days).

Latest-close fetch uses `range=5d` and takes the last non-null row.

Network timeout 15s, matching existing skills. Any exception → return `None`, caller emits degraded output.

### 5.5 Backfill and History Contract

**Minimum viable history:** 20 rows per symbol. Below 20 the skill emits `[WARN: insufficient history for <symbol>]` and runs degraded. 20 is enough to compute a meaningful max/min even if the full year isn't available; anything less and the high/low values are too noisy to be useful.

**First run per symbol** (`oil_daily` has zero rows for that symbol): call Yahoo with `range=1y` (returns up to ~252 trading-day rows, fewer if Yahoo doesn't have a full year — e.g. newly listed instruments or Yahoo outages). Insert everything returned. Proceed if ≥ 20 rows, degrade otherwise.

**Subsequent runs:** fetch latest close only (`range=5d`), upsert. Window is computed by `SELECT ... ORDER BY date DESC LIMIT 252`.

**No gap repair.** If the cron misses a day, that day's close is simply absent. The skill does not re-fetch history to patch holes. Gaps at daily frequency don't distort min/max materially because the min and max of a set are insensitive to the exact row count above 20.

**"252" is a ceiling, not a guarantee.** The spec does not promise 252 valid closes; it promises "the most recent up to 252 rows, minimum 20." A run with 180 rows (e.g. six months in, a couple of cron gaps) is healthy. A run with 19 rows is degraded.

## 6. CLI Surface

Mirrors `doughcon`:

```
python3 ~/a/claw-skills/oilcon/scripts/run.py                    # deliver (default)
python3 ~/a/claw-skills/oilcon/scripts/run.py --mode record
python3 ~/a/claw-skills/oilcon/scripts/run.py --deliver-to CHAT  # override target
```

Modes:

- `--mode deliver` (default) — compute snapshot, format body (healthy or degraded per §7), pass to `deliver_or_fail`. Exit code is delegated to `delivery.py`: 0 on successful send (or successful stdout print when no `--deliver-to` given), 1 on Telegram send failure.
- `--mode record` — compute current snapshot, append to `~/.nullclaw/oilcon-history.log`, exit non-zero on failure (so cron `last_status` catches gaps, same contract as doughcon)
- `--deliver-to CHAT_ID` — send to specific Telegram chat instead of stdout
- `--account NAME` — telegram account selector (default `main`), matching `stock`

Record-mode log line:

```
2026-04-15 17:00:01 CST  WTI 78.20  high 92.40@2026-01-08 (-15.4%)  low 74.10@2026-03-02 (+5.5%)  BZ +0.7% HO +0.8%
```

## 7. Error Handling

Two dimensions that must not be conflated: **body health** (what the message says) and **process exit** (what the cron scheduler sees). Delivery failure *always* fails the process, per `lib/delivery.py`'s contract — the skill has no opt-out.

| Failure | Body | Deliver-mode exit | Record-mode exit |
|---|---|---|---|
| Turso creds missing | Degraded: `[WARN: turso credentials missing]` | 0 if delivered, else `delivery.py` exits 1 | Non-zero |
| Turso connection fails | Degraded: `[WARN: turso unavailable]` | 0 if delivered, else `delivery.py` exits 1 | Non-zero |
| Yahoo fetch fails on first-run backfill | Degraded: `[WARN: history fetch failed for <symbol>]` | 0 if delivered, else `delivery.py` exits 1 | Non-zero |
| Yahoo fetch fails for latest close, but stored history ≥ 20 rows for WTI | **Healthy body** using the most recent stored close for the failing symbols, with `(stale)` appended to each affected symbol's "current price" line. Confirmation marks still computed using the stale symbols' last-available change. | 0 if delivered, else `delivery.py` exits 1 | Non-zero |
| Stored history < 20 rows for WTI after any backfill attempt | Degraded: `[WARN: insufficient WTI history (<n> rows)]` | 0 if delivered, else `delivery.py` exits 1 | Non-zero |
| Stored history < 20 rows for Brent or HO (but WTI healthy) | **Healthy WTI body**, with `Brent n/a` or `HO n/a` in the confirmation line instead of `✓`/`✗` | 0 if delivered, else `delivery.py` exits 1 | Non-zero (if the failing symbol was the one the run was recording) |
| Telegram send fails | Body (healthy or degraded) is preserved on local stdout by `delivery.py`; stderr gets a `[delivery]` diagnostic | **1** — `delivery.py` calls `sys.exit(1)` and trace markers are never emitted | N/A (record mode never calls `deliver_or_fail`) |

**Latest-fetch contradiction resolved:** a fetch failure during first-run backfill is degraded (no stored history to fall back on). A fetch failure on a subsequent run, when the symbol already has ≥ 20 stored rows, is *healthy-with-staleness*. These are different cases and the table now says so explicitly. There is no "any fetch failure → degraded" catch-all.

**Sign rule for confirmation marks:** a move is treated as positive iff the raw (unrounded) percent change is strictly > 0, negative iff strictly < 0, and flat iff exactly 0. A flat confirmation symbol renders as `–` (en dash), not `✓` or `✗`, regardless of WTI's direction. In practice a daily close change of exactly 0.00% is extremely rare; this rule exists to make the behavior defined, not because it matters often. The displayed percentage is rounded to one decimal place for humans but the sign is computed from the unrounded value, so a displayed `+0.0%` that was actually `+0.03%` is still a `+` for confirmation purposes.

No retries at the skill layer. Nullclaw cron's `retry_once` repair policy handles transient failures at the scheduler layer, per project convention. Marker emission (`emit_skill_status`, `emit_trace`) happens *after* `deliver_or_fail` returns, per `trace_marker.py`'s module docstring: "Call the marker helpers only after delivery confirmation so delivery failures remain hard exec errors instead of semantic verification failures." This is load-bearing — do not reorder.

## 8. Testing

Unit tests under `~/a/claw-skills/lib/test_oil_fetch.py` and `test_oil_store.py`, matching existing `test_delivery.py` / `test_telegram_retry.py` pattern. Tests:

- `oil_fetch.parse_chart_response` — fixture JSON from Yahoo, verify (date, close) extraction, null handling, symbol encoding
- `oil_store.ensure_schema` — idempotent on repeated calls
- `oil_store.needs_backfill` — true when empty, false when any rows present
- `oil_store.window` — returns rows ordered correctly, respects limit, handles short history
- `oil_store.upsert` — replaces existing row for same (symbol, date)
- `run.format_message` — fixture data in, expected Chinese string out, including `✓`/`✗` cases and degraded variant

Turso tests use an in-memory libsql connection (`:memory:`) — no real network credentials needed in CI. Yahoo fetch tests use recorded fixture JSON files, never hit the network.

No integration test against real Yahoo or real Turso in this design. If the user wants a smoke test, `run.py --mode deliver --deliver-to <test-chat>` run manually is the verification path.

## 9. Scheduling

Not part of this design. The user will create a cron entry through nullclaw's existing cron subsystem after the skill is working. Recommended cadence: once per US trading-day evening (e.g. `0 22 * * 1-5` in user's timezone). Cron registration will reference the skill's `run.py` path and use the standard `skill_contract` with `retry_once`, identical to doughcon's contract.

## 10. Out of Scope

- JETS price data, JETS correlation, any airline equity integration
- VIX or any macro filter
- Multi-timeframe analysis (weekly, monthly rollups)
- Backtesting, historical signal evaluation
- Alerting on thresholds, transitions, or shocks
- Charts, images, or non-text output
- Supporting instruments beyond CL=F, BZ=F, HO=F
- A generic "eventcon" framework (deferred until a 3rd stateful skill actually appears)
- Migrating `doughcon` to share any of the new `lib/` modules (doughcon is stateless and has no reason to use `oil_store.py`)

## 11. Open Questions

None at spec time. All design decisions have been locked through brainstorming. If any surface during implementation, the implementer should pause and ask rather than guess.

## 12. Success Criteria

1. Running `python3 ~/a/claw-skills/oilcon/scripts/run.py` on a machine with valid Turso credentials and network access produces a well-formed Chinese-language Telegram message containing WTI current, high, low, days, percentages, and Brent/HO confirmation marks.
2. First run backfills 252 days of history for all three symbols without manual intervention.
3. Subsequent runs complete in under 5 seconds (one Yahoo call per symbol + a few Turso queries).
4. All unit tests pass under `python3 -m pytest ~/a/claw-skills/lib/` (or whatever the repo's existing test invocation is).
5. Skill gracefully degrades (never crashes cron) when Turso or Yahoo is unreachable.
6. The user reads the message daily for several weeks and, at least once, says "the turn looks like it might be in" based on what the skill is showing — i.e. the message contains enough information to support the user's actual decision. This is the real test.
