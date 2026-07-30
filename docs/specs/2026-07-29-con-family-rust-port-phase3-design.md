# Phase ③ — chipcon, oilcon, inflation-con to Rust

Status: design, revision 4 (after three Codex reviews)
Date: 2026-07-29

Phases ① (claw-core + doughcon) and ② (weather) are shipped and live. This phase
ports the remaining `con` family, retires the last two Python shared libs, and
consolidates oil storage into `price-registry`.

**Revision history.** Rev 1 was reviewed and returned 8 BLOCKERs, all corroborated
against source. Rev 2 fixed one outright and seven partially. Rev 3 closed most gaps; rev 4 fixes the four blockers the third review found —
including two claims about chipcon that were simply false. What rev 2 had wrong:

- the backfill guard measured row count, which is not what the analysis needs;
- the `chart.error` mapping is **not implementable** against `market-fetch`'s
  current error enum without swallowing network failures;
- "keep the literals" is wrong for FRED — the literal is already broken in
  production.

## The three skills are less alike than rev 1 and rev 2 assumed

Every claim in this table was read from source, not inferred.

| | oilcon | inflation-con | chipcon |
|---|---|---|---|
| failure model | **all-or-nothing** — a history-fetch exception aborts the whole symbol loop | per-series tolerant; hard-fails only if `core_pce` empty | per-symbol tolerant; hard-fails only if `SMH` empty |
| persistent state | Turso + `~/.nullclaw/oilcon-history.log` | `~/.nullclaw/inflation-con-history.log` | `~/.nullclaw/chipcon-history.log` — no market-data store; refetches 1y per run into memory |
| job ID | wrapped in backticks | unquoted | unquoted |
| `parse_mode` | default | `None` | `None` (comment explains why: status names contain underscores) |
| record mode | rejects warned/stale runs | **accepts** warnings, records them, emits degraded | accepts warnings, emits degraded — like inflation-con |
| cadence | weekdays 22:00 | 3×/month, 06:00 on the 3rd–5th | Tue–Sat 05:30 |

Consequences that change the design:

- **A throwing fetch adapter is safe for chipcon and inflation-con, and unsafe for
  oilcon.** The first two catch per-item and degrade locally. oilcon re-raises a
  history failure as `ValueError`, which `build_snapshot` converts into
  `Snapshot(symbols={}, warning=...)`, discarding symbols already built.
- **Three separate cutovers, not one.** They share no state, fail differently, and
  run on three different cadences. inflation-con's "observe several runs" is a
  month, not a week.
- **chipcon has no market-data store, but it is not stateless.** Rev 3 claimed "no
  history file" and "record mode: n/a". **Both were false** — `--mode record` exists
  and appends `~/.nullclaw/chipcon-history.log`. The error came from reading only the
  functions a grep surfaced; `main` was never read, and a grep for `append` was
  drowned by `reasons.append(...)`. Absence in a noisy search is not evidence.
- **chipcon still carries the least migration risk** — nothing to move to Turso — but
  it has the same record/history obligations as the other two.
- **A latent bug to decide on, not inherit silently**: the fallback
  `deliver_or_fail(deliver_to, output, account="main")` takes no `parse_mode`, yet
  `emit` calls it with `parse_mode=None`. A manual run outside nullclaw (where the
  import fails) raises `TypeError` instead of printing the report. The port either
  preserves this explicitly as a wart or fixes it as a sanctioned change — silently
  "fixing" it in translation would be an undeclared behaviour change.

## `chart.error`: the mapping rev 2 proposed cannot be written

`market_fetch::yahoo::FetchError` has exactly three variants:

```rust
pub enum FetchError { Http(String), Parse(String), NoData }
```

`chart.error` produces `Http(err.to_string())`. A ureq transport failure in
`fetch_history` also produces `FetchError::Http(e.to_string())`. **They are
indistinguishable by variant**, so oilcon cannot "match only the semantic
`chart.error` case" — any such match swallows real network failures too, converting
an outage into a silent empty series.

Two ways forward. **Decision: (A).**

**(A) Add a distinct variant to `market-fetch`.**

```rust
pub enum FetchError { Http(String), Upstream(String), Parse(String), NoData }
```

`parse_yahoo_chart` returns `Upstream` for a non-null `chart.error`; transport
failures stay `Http`.

**`Upstream` alone is not enough.** Python's `parse_chart_response` also returns `[]`
when `chart.result` is absent or `closes` is falsy; Rust returns `NoData` for those
same shapes. Letting `NoData` through would convert an empty-history payload into
oilcon's whole-snapshot failure — the exact regression this section exists to prevent.

The oilcon and chipcon adapters therefore map **both `Upstream` and `NoData` → empty
series**, and let `Http` and `Parse` through as errors. Parity tests are written per
payload shape (no `result`, empty `timestamp`, null `closes`, `chart.error`), not just
for `chart.error`.
This is a change to a shipped crate, so it carries market-fetch's own gate: its 17
Yahoo tests must still pass, and `chart_error_is_an_error_not_an_empty_series` needs
updating to assert `Upstream` rather than `Http` — the one place in this phase where
an existing test legitimately changes, recorded here so it is not mistaken for
drift.

**(B) Rejected: a `{rows, warning}` result type and a per-symbol warning channel.**
That is a real improvement and a real behaviour change: oilcon has no
successful-snapshot-plus-warning path, and setting `Snapshot.warning` flips status to
degraded *and* makes record mode reject the run. It needs its own status, formatting,
record-mode and golden-test decisions. It is a follow-on change, not part of a port.

**The diagnostic is deliberately given up.** With (A), a delisted or renamed oil
symbol surfaces only indirectly — as `n/a`, stale, or insufficient history. That is
what happens today. Saying so plainly is better than rev 2's contradiction, which
promised both exact parity and a recorded warning.

## inflation-con carries a reachable crash and a piece of dead code

Both found 2026-07-30 while preparing Plan 2, both reproduced rather than reasoned
about, and both need a decision before that plan can be written.

### The crash

`classify`'s YELLOW branch builds this string when the levels reach RED but the
context clause does not hold:

```python
f"(breakeven {be_last[1]:.2f} < 2.5% and not clearly rising, or stance easing)"
```

`be_last` is `latest(breakeven)`, which is `None` when the breakeven series is empty.
Reproduced with core-PCE 3-mo and 6-mo both ≈3.6% and `breakeven_10y = []`:

```
TypeError: 'NoneType' object is not subscriptable
```

**This path is reachable today.** `fetch_all` is per-series tolerant — a failed
`T10YIE` fetch appends a warning and stores an empty list rather than raising. So
"inflation runs hot enough to reach the RED levels" plus "the breakeven fetch failed
that morning" crashes the run, which `main` then catches and reports as
`INFLATION-CON failed`.

It is hard to notice precisely because it needs two independent conditions at once:
high inflation *and* a fetch failure. Neither alone shows it.

### The dead code

Inside the same YELLOW branch:

```python
if not core_cpi_hot_3_or_6:
    reasons.append("core CPI not confirming yet")
```

YELLOW's own entry condition **requires** `core_cpi_hot_3_or_6` to be true, so the
negation can never hold. That sentence has never been printed.

### Resolved 2026-07-30: the Python was fixed first

Of the three options — reproduce the crash faithfully, fix only in the port, or fix
the Python first — the third was chosen and applied. A differential against an oracle
that crashes only proves both sides crash, so Plan 2's oracle had to be correct before
it could mean anything.

The fix builds the breakeven fragment separately and falls back to
`"breakeven unavailable this run"` when `be_last` is `None`. Four regression tests were
added, and they deliberately cover more than the crash itself:

| test | what it holds |
|---|---|
| `no_crash_when_levels_reach_red_and_breakeven_is_empty` | the crash, **and** that the note is still produced |
| `breakeven_note_reports_the_value_when_it_is_present` | the working path still prints the real number, not a placeholder |
| `breakeven_shorter_than_the_lookback_still_classifies` | fewer than 64 observations gives `None`, not an error |
| `red_still_requires_breakeven_data_or_a_rising_trend` | an empty series must not count as RED confirmation |

The second and fourth exist because guarding a `None` is only correct if it neither
degrades the case that already worked nor loosens the judgement. With only the first
test, deleting the whole note would also have passed.

Baseline before the fix: 30 passed, 3 failed. After: 32 passed, 1 failed — the
remaining failure is `test_chipcon_remains_copy_when_present`, a stale scope guard
from an earlier workstream that has been failing since `~/.nullclaw/skills/chipcon`
became a symlink on 2026-07-13. Unrelated, and not touched here.

**Caveat on how that was measured.** This environment has no working `pytest` — the
shim on `PATH` cannot import the module. The 33 tests were run through a minimal
stand-in implementing only `approx`, `fail`, `raises`, `skip`, `monkeypatch` and
`tmp_path`, which is all this file uses. That is not pytest, and a real pytest run has
still not happened.

The dead code should simply be dropped in the port, with a line in the
intentional-differences record. It cannot change behaviour, and carrying it forward
would leave the next reader of the Rust wondering what they are missing.

## Storage consolidation into `price-registry`

`oilcon.oil_daily(symbol, date, close)` and `price-registry.prices(ticker, date,
close, source)` are the same table with different column names, and `price fetch
CL=F` already stores a year of daily closes to `prices`.

### Mapping

| `oil_daily` | `prices` | note |
|---|---|---|
| `symbol` | `ticker` | value unchanged — `CL=F` stays `CL=F` |
| `date` | `date` | ISO, unchanged |
| `close` | `close` | unchanged |
| — | `source` | oilcon writes `"yahoo"` |

### Live-table audit — mixed sources are already real

Run 2026-07-29 against `price-registry`:

```
JETS  yahoo           251   2025-07-08..2026-07-07
MSFT  yahoo           251   2025-07-09..2026-07-08
QQQ   yahoo           252   2025-07-08..2026-07-08
SGOV  yahoo           251   2025-07-08..2026-07-07
SMH   stooq             4   2026-06-01..2026-06-16
SMH   stooq-intraday    1   2026-06-08..2026-06-08
SOXX  stooq             2   2026-06-02..2026-06-03
SPCX  yahoo            17   2026-06-12..2026-07-08
```

Two facts that settle open questions:

1. **A single ticker's history already spans multiple sources.** `SMH` interleaves
   `stooq` and `stooq-intraday` by date. So "only two writers, both writing yahoo" is
   false as a general statement about this table — it is true only of the three oil
   tickers, and only because they have no rows yet.
2. **`CL=F`, `BZ=F` and `HO=F` have no rows at all.** oilcon's first run after
   cutover performs three full backfills. Backfill correctness is the critical path,
   not an edge case.

### Provenance policy

Keep `PRIMARY KEY (ticker, date)`. Adding `source` to the key would permit multiple
rows per date while `read_latest`, `read_at` and `read_history` neither filter nor
order by source — reads would become ambiguous and history could contain duplicate
dates. That is worse than the problem it solves.

Instead:

- `"yahoo"` is the **canonical source for the three oil tickers**; oilcon writes it,
  matching `price-cli`'s `upsert(&conn, t, &q.date, q.close, "yahoo")` (`run.rs:64`).
- `read_window` returns `Vec<StoredPrice>`, which carries `source`. Provenance stays
  visible even though today's caller ignores it. A tuple would make a future
  multi-source bug invisible.
- `upsert` is public and accepts any source, so this is **convention, not
  enforcement**. The audit above is the evidence that convention is not enough
  in general; it is accepted here only because these three tickers have a single
  writer.
- **oilcon validates what it reads.** Convention is only safe if something checks it,
  so every row `read_window` returns for an oil ticker must have `source == "yahoo"`;
  a mismatch triggers a Yahoo repair backfill, not a silent calculation over foreign
  data. Without this check `Vec<StoredPrice>` buys visibility and nothing else,
  because the caller would discard `source` immediately.
- Note the limit of the policy: `StoredPrice` shows the **surviving** attribution
  only. When a `stooq-intraday` row overwrites a `stooq` row on the same date, the
  earlier provenance is gone. That is inherent to a canonical-value table and is
  accepted, not solved.
- The oil tickers do not have a single *writer* — `price-cli` and oilcon can both
  write them. They have a single intended canonical *source*. Those are different
  claims and rev 3 conflated them.
- A second source for these tickers is the trigger to design source selection —
  which is a reader-side problem, not a primary-key problem.

### Backfill: completeness, not row count

Rev 2 proposed `read_window(ticker, 252).len() < 70`. **That is the wrong measure**,
for three reasons found in review and confirmed against `run.py`:

- `compute_extremes` scans **every** row in the window via `max/min(enumerate(rows))`.
  At 70 rows the reported WTI high/low becomes a 70-day extreme instead of a
  one-year one, and `days_since_high` / `days_since_low` change with it. The
  threshold was chosen for `classify_oil_trend` (which needs 70) while a different
  consumer needs the full window.
- A chunked 252-row write that commits 200 rows and then fails leaves 200 > 70, so
  the guard reports "complete" and the missing rows are never fetched.
- Count says nothing about continuity or recency: 70 sparse or ancient rows pass.

Replacement, three parts:

**1. The batch write is transactional across ALL chunks.** `upsert_many` either
applies every row or none — chunking is an implementation detail inside one
transaction, not a sequence of independent commits. The `price_backfill` marker is
written in that same transaction. A transactional batch followed by a separate marker
write is still wrong: the marker could commit without the rows.

**Stale-refresh fallback.** Freshness introduces a path that did not exist before: an
established symbol that goes 8 days stale now enters history backfill, and if that
request fails, oilcon's history-error path discards stored rows that were previously
usable. **Decision: a failed refresh on a symbol that already has stored history falls
back to the stored rows and marks the symbol stale**, matching what a failed
`fetch_latest` does today. Only an empty store may hard-fail.

**2. The guard is span-and-freshness, not count.**

```
needs_backfill(ticker) =
      window.is_empty()
   || window.rows.len()   < MIN_ROWS                    (70 — analytic sufficiency)
   || window.newest_date  < today - MAX_STALE_DAYS      (7 — freshness)
   || window.span_days    < MIN_SPAN_DAYS               (300 — horizon coverage)
```

All three conditions are needed and they answer different questions. **Count alone**
misses staleness and sparseness; **span alone cannot reject sparse data** — two rows
365 days apart satisfy it while failing every observation threshold. 70 is analytic
sufficiency (the `classify_oil_trend` floor); 300 days is *horizon coverage* for the
one-year extrema `compute_extremes` reports, derived from the calendar year the window
represents rather than from the 20/50/70 thresholds — stated as judgement, not
calculation. 7 days spans a long weekend plus holidays.

Span, because the analytic window is a calendar year and `compute_extremes` reads all
of it. Freshness, because every consumer indexes from the tail, so a gap at the
newest end is the one that changes output.

**3. A backfill marker ends the retry loop for genuinely short series.** A ticker
whose upstream history is shorter than `MIN_SPAN_DAYS` would otherwise refetch a year
on every run, forever. A small table records the attempt:

```sql
CREATE TABLE IF NOT EXISTS price_backfill (
  ticker      TEXT PRIMARY KEY,
  attempted_at TEXT NOT NULL,   -- ISO date of the last successful backfill write
  rows_written INTEGER NOT NULL,
  span_days    INTEGER NOT NULL -- what upstream actually returned
)
```

The marker is a **retry throttle, not proof of completeness**, and its semantics must
be pinned or it rots:

- identity is `(ticker, source, requested_range)` — a marker written under one source
  policy must not suppress repair after the policy changes;
- written only after a **successful, non-empty** response, and **inside the same
  transaction as the rows**, so a marker can never outlive the data it claims;
- never created from an `Upstream`/`NoData` empty mapping — that is absence of data,
  not evidence of a short series;
- carries a TTL, because a young series gains history; expiry forces a re-attempt.

For these three long-lived oil contracts the marker is not strictly required. It is
included because `price-store` is shared infrastructure and the next consumer may
track a genuinely short series.

**Cost note.** `read_window(252)` transfers more than the old `SELECT 1`. That is the
price of a guard that means something; 252 rows once per symbol per run is not a
concern against a 22:00 daily schedule.

## `price-store` extraction boundary

`store.rs` is not only a price store: `ensure_schema` creates `prices`, `config` and
`credit_spreads` in one function, and the file also holds config CRUD and all
credit-spread operations.

**Move only the price surface**: `StoredPrice`, the `prices` DDL + index, `one`,
`upsert`, `upsert_many` (new, transactional), `read_latest`, `read_at`,
`read_history`, `read_window` (new), plus the `price_backfill` table and its
accessors. Config and credit-spread code stay in `price-cli`, whose `ensure_schema`
composes: call `price_store::ensure_schema`, then create `config` and
`credit_spreads`.

Composition is required: `tests/price.rs::ensure_schema_does_not_seed_config` asserts
the `config` table exists and is **empty** after `ensure_schema`.

### The shim is confirmed at the import level

`tests/price.rs` imports `ensure_schema, upsert, read_latest, read_at, read_config,
read_history, set_config` (line 78) and `list_credit_series, read_credit_history,
upsert_credit_many` (line 177 — both groups are in `price.rs`; `tests/credit.rs`
imports neither). Every one is `pub`; nothing touches the private `one` or
`CREDIT_UPSERT_CHUNK`. A selective re-export shim therefore preserves both files.

Import compatibility is proven; **full test compatibility is proven only by running
them**, and the gate is unchanged SHA-256 on both files plus a green suite.

### `read_window`

```rust
pub async fn read_window(conn: &Connection, ticker: &str, limit: i64)
    -> Result<Vec<StoredPrice>>   // ascending by date; the last `limit` rows
```

`ORDER BY date DESC LIMIT ?` then reverse, matching `oil_store.window`. `limit <= 0`
returns an empty vec without issuing SQL.

## Configuration: literals, except the one that is already broken

Rev 2 accepted "keep the Yahoo and FRED literals in both ports". **For FRED that is
now wrong.**

Measured 2026-07-29: FRED refuses `nullclaw/1.0` — and refuses it by hanging the
connection rather than returning 4xx, so the symptom is a timeout that looks like a
network fault. `curl/8.5.0 nullclaw/1.0`, `python-urllib/3.11 nullclaw/1.0` and
`Wget/1.21 nullclaw/1.0` all succeed; matching is on the leading token.

inflation-con's last successful run was 2026-07-08 and it runs 3×/month, so the
breakage sat unnoticed for three weeks. The Python has been fixed to
`curl/8.5.0 nullclaw/1.0`; **the port must carry the fixed literal, not the
historical one.** "Preserve current behaviour" is only safe when the current
behaviour works, and nobody had measured it.

Yahoo still accepts `nullclaw/1.0` (verified same day), so chipcon and oilcon keep
theirs. Neither skill reads config from the DB: `ensure_schema` creates the `config`
table empty by design, and adding a config-missing failure mode to a first-run
backfill path buys nothing here.

## FRED's default window — measured, not assumed

Rev 2 justified "extra history changes nothing" by arguing every lookback sits inside
FRED's ~3-year default window. **That premise was wrong.** Measured row counts, with
and without `cosd=1900-01-01`:

| series | rows without `cosd` | with `cosd` |
|---|---:|---:|
| PCEPILFE | 809 | 809 |
| CPILFESL | 834 | 834 |
| PCEPI | 809 | 809 |
| CPIAUCSL | 954 | 954 |
| T10YIE | 6149 | 6149 |
| DFII10 | 6148 | 6148 |
| DGS10 | 16845 | 16845 |

The payloads are **identical**. The ~3-year cap applies to the licence-restricted
ICE/BAML series used by `price cds`, not to these. So always sending `cosd` changes
nothing for inflation-con — for a simpler reason than rev 2 gave, and one that also
protects `details["core_pce_obs"]`, which is rendered in the `INSUFFICIENT_DATA`
message and always appears in the history-log record line.

## nullclaw contract — per skill, because they differ

Phase ① established that the marker lines are a hard scheduler contract. The three
skills emit them differently, so each needs its own golden tests.

**oilcon**: append `NULLCLAW_JOB_ID` in backticks → `deliver_or_fail` →
`emit_skill_status` → `emit_trace`. Record mode emits both markers **only after** the
history append succeeds; a warning in record mode, or a write failure, exits non-zero
**without** markers. `~/.nullclaw/oilcon-history.log` format from
`format_record_line`.

**inflation-con**: unquoted job ID, `parse_mode=None`. Record mode **accepts** a
warned run, records it, and emits degraded markers — the opposite of oilcon. Delivery
failure is caught by `main`, followed by failed markers. Its own
`~/.nullclaw/inflation-con-history.log`. `load_config` runs **outside** `main`'s
`try`, so a malformed config produces neither the designed markers nor controlled
output — existing behaviour, preserved, and flagged as a known wart.

**chipcon**: unquoted job ID, `parse_mode=None` (the source comment explains why:
status names contain underscores and a fetch warning can carry unbalanced brackets,
which break Telegram's legacy Markdown). No history file.

Golden tests pin, per skill: exact marker text, ordering relative to delivery, stdout
vs stderr, exit code per branch, job-ID presence and quoting, and delivery-failure
behaviour.

## Cutover — three independent sequences

`price-store` first, because both other sequences depend on it.

### 0. `price-store` extraction (`price-cli` behaviour unchanged)

Gate: both existing test files pass with unchanged SHA-256;
`RUSTFLAGS="-Dwarnings" cargo test` green. Then exercise the **extracted surface**,
not an unrelated one: `price fetch <ticker>` and `price read`/`history`, plus one
`finance-cli value` run that reads `prices`. Rev 2 proposed a `price cds fetch` smoke
test, which mostly exercises code that stays in `price-cli`.

### 1. chipcon (lowest risk — no storage)

Port with tests first. Preflight: run the Rust binary with `--deliver-to` omitted and
compare its rendered message against the Python's for the same day. Switch the
`SKILL.md` line. **Accept** after 3 consecutive scheduled runs with `status=ok` and
byte-identical classification. **Roll back** by reverting one `SKILL.md` line.

### 2. inflation-con (monthly — the slowest feedback)

Same shape. **Accept** after one scheduled run on the 3rd–5th with `status` matching
the Python's for the same inputs and a history-log line of the same shape. Because
the cadence is monthly, keep the Python entry point for a full cycle. Do not treat
"three runs" as available here.

### 3. oilcon (highest risk — storage swap)

1. Provision and verify the `price-registry` write credential out of band.
2. Preflight in **`--preflight` mode** — see below — writing to `price-registry` and
   delivering nowhere. Confirm all three tickers reach `MIN_SPAN_DAYS` of span with a
   newest date matching the Python's, and that the rendered message matches the
   Python's for the same day.
3. Switch the `SKILL.md` line.
4. **Accept** after 5 consecutive scheduled runs with `status=ok`, a history-log line
   of the same shape, and `compute_extremes` output within rounding of the Python's.
5. **Roll back** by reverting `SKILL.md`; `TURSO_DATABASE_URL`/`TURSO_AUTH_TOKEN` and
   the `oilcon-yanggf8` database stay untouched throughout.

### `--preflight` does not exist and must be built

No inlined CLI has a "write storage but deliver nowhere" mode. It is required for
step 2 and must be explicit: writes to `price-registry`, prints the message to stdout,
performs **no delivery**, writes **no history line**, and emits **no markers**. A test
asserts all five.

It is **not** called `--dry-run`. That name promises no side effects while the mode's
entire purpose is to write production data; the misnomer is exactly how someone runs
it believing it is safe.

**Acceptance is parity, not `status=ok`.** A legitimate upstream warning must not read
as a failed port. Each cutover compares the Rust and Python outputs **on identical
inputs** — captured upstream responses replayed against both, with time frozen —
comparing normalised domain fields rather than rendered text, which carries
timestamps. Record-mode behaviour is checked by explicitly invoking record mode; a
scheduled deliver-mode run never writes the history line.

**Rollback triggers**, not just rollback actions: classification differs from Python
on identical inputs; a Rust-only non-zero exit; marker text, ordering or exit code
differing from the Python golden; stored coverage below the backfill guard after a
run that reported success; or a credential that cannot be renewed before expiry.

## Credentials — unattended is the hard part

Today: `TURSO_DATABASE_URL` + `TURSO_AUTH_TOKEN` from `~/.nullclaw/.env`, pointing at
`libsql://oilcon-yanggf8`. Replacement:

- **Database**: `price-registry`, located the way `price-cli` locates it
  (`PRICE_TURSO_URL`).
- **Scope**: write — oilcon backfills.
- **Resolution**: `turso-util` cached-or-mint, cache
  `~/.config/gwebcdb/tokens/price-registry-write.json`. `resolve_cached_or_mint`
  refuses caches with unsafe permissions, so the file must be `0600` and owned by the
  scheduler user.
- **The unattended hazard, stated as a requirement rather than a note**: minting
  shells out to the `turso` CLI and needs an active `turso auth login`. A cron host
  cannot perform an interactive login. Therefore the cache must be provisioned with a
  lifetime longer than the observation window, and expiry must be detected *before*
  it bites — a `price-cli doctor`-style check that reports token expiry is the
  existing mechanism and should be run as part of acceptance.
- **Failure behaviour**: no usable credential → degrade with a warning, exactly as
  `MissingCredentialsError` does today. Never a silent skip.
- **Redaction**: tokens never reach stdout, the history log, or a delivered message.

## Tests

The 68 existing tests port as the oracle. Ownership:

| group | destination | delta |
|---|---|---|
| `chipcon/scripts/test_run.py` (11) | `crates/chipcon` | none |
| `lib/test_oilcon_run.py` (21) | `crates/oilcon` | none — `chart.error` maps to today's behaviour via `FetchError::Upstream` |
| `inflation-con/scripts/test_run.py` (29) | `crates/inflation-con` | none — `cosd` measured inert |
| `lib/test_oil_fetch.py` (3) | `market-fetch` | retire only after mapping each case to an existing Yahoo test by name |
| `lib/test_oil_store.py` (4) | `price-store` | rewrite against libsql; the Python fixtures are sqlite3 |

One existing Rust test changes: market-fetch's
`chart_error_is_an_error_not_an_empty_series` must assert `Upstream` instead of
`Http`. That is the only sanctioned edit in this phase.

New tests, none of which exist today:

- transactional `upsert_many`: a failure mid-batch leaves **zero** rows;
- `needs_backfill` at the boundaries — empty, stale-but-long, fresh-but-short, sparse,
  and a genuinely short upstream series that must not refetch forever;
- `price_backfill` marker written, read, and honoured;
- `read_window` with mixed `source` values (the live table already has them);
- per-skill nullclaw goldens: marker text, ordering, exit codes, job-ID quoting,
  `parse_mode`, and each record-mode branch;
- credential failure branches: missing, expired, unsafe permissions, mint unavailable.

The plan carries the test-by-test matrix; this design names the groups and every
known delta.

## Risks

- **oilcon's storage swap** is the riskiest change. The span-and-freshness guard plus
  a transactional batch is what makes a failed backfill recoverable; rev 2's count
  guard was not.
- **`price-cli` is live.** The extraction takes the market-fetch gate: existing tests
  pass with zero edits or the extraction is wrong.
- **Writes are not snapshot-atomic across symbols.** oilcon processes WTI → Brent →
  HO, writing each before the next starts. A Brent failure leaves WTI's writes
  committed while the snapshot is discarded. Existing behaviour, preserved; the
  completeness guard makes the leftover harmless.
- **inflation-con's monthly cadence** means a regression can hide for a month — as
  the User-Agent breakage just did.

## Out of scope

The remaining daily skills and `news` (3,534 lines).

**Retiring `oilcon-yanggf8`** — it is the only rollback dataset, since history is not
migrated. It stays, with its token, until oilcon's acceptance criteria above are met
and the old and new 252-row windows have been compared for all three symbols.
Retirement is a separate approved step.

**`cds-con`** is deferred to after this phase. `price cds fetch/show` exists — data
layer and read command, 8 FRED series — but the *con* half was never built: no
skill, no schedule, no delivery. Measured cost: on 2026-07-29 the daily series' last
row was 2026-07-24, because nothing runs the fetch. When built it reuses this phase's
output, and must not print ICE and Moody's percentiles side by side without saying
that the ICE ones cover ~3 years.
