---
name: cds-con
description: Report corporate credit-spread levels and their percentile position within stated windows; signal-only, deliberately unclassified (no status ladder, no cheap-or-expensive label).
always: true
---

# cds-con

Report **corporate borrowing cost** — what a bond buyer is paid for taking
credit risk — as levels and percentile ranks. This is **observation-only**: no
entry/exit advice, no position sizing, no portfolio edits.

## This skill has no `狀態：` line, and that is deliberate

Every other skill in this repo opens with a classified status. cds-con does
not, and it must not be "fixed" to add one.

A percentile is a rank **within a stated window**, and the window can flip the
conclusion. `baa10y` on 2026-07-31 sat at p14.8 over one year, p10.2 over ten,
and p11.2 over the whole store — a ladder would have to pick one and would be
reporting an artefact of that choice. The same rule already governs
`price cds show`, which emits no verdict either, and it is not weakened by this
message being pushed rather than pulled.

The message lays out levels, percentiles and coverage. The reader judges.

## Script

```
~/.nullclaw/skills/cds-con/bin/cds-con
```

## Usage

```
~/.nullclaw/skills/cds-con/bin/cds-con
~/.nullclaw/skills/cds-con/bin/cds-con --deliver-to CHAT_ID
```

`--mode` accepts only `deliver` (also the default). Fetching is a separate job
— see **Cron** below.

## What it reads, and the trap in it

Two families that are **not commensurable**, rendered in separate blocks:

- **Spreads** (`baa−aaa`, `baa10y`, `hy_oas`, `ig_oas`) — compensation for
  credit risk, with the risk-free rate already removed.
- **Yields** (`aaa`, `baa`, `hy_yield`, `ig_yield`, `ccc_yield`) — total
  borrowing cost, which **includes** the risk-free rate.

On 2026-07-31 the yields sat near one-year highs while the spreads sat near
multi-decade lows. That is a fact about interest rates, not about credit
stress, and comparing a yield percentile against a spread percentile
manufactures a signal that is not there. The blocks and their labels exist to
prevent exactly that.

**Coverage is mixed inside each family**, so it is printed on every series line
rather than a block header: `aaa`/`baa` reach 1919, `baa10y` 1986, and the five
ICE/BAML series only ~3 years — a licensing cap on the keyless FRED endpoint,
not a bug. `baa−aaa` is derived at read time from `baa` and `aaa`; it is the
longest-history credit-stress measure available.

## Data Store

- Source: FRED keyless CSV, stored in the shared `price-registry`
  (`credit_spreads` table), written by `price cds fetch`.
- Read through `credit-store`, the same crate `price-cli` uses — one percentile
  implementation, not two.
- **This skill never fetches.** It reads what the fetch job stored, and states
  on the `資料:` line how old that is.

## Cron

Two jobs, deliberately separate. A FRED outage then affects freshness only —
the message still goes out and says honestly how stale the data is.

```
nullclaw cron add "0 6 * * 2-6" "$HOME/b/gwebcdb/target/release/price cds fetch" --timeout 300 --tz +08:00
nullclaw cron add-skill "30 6 * * 2-6" cds-con --deliver-to 7972814626 --timeout 180 --tz +08:00 --verify skill_contract --repair retry_once
```

The fetch runs first and the deliver half an hour later, so a normal morning
reads data written minutes earlier. They are not ordered by anything stronger
than the clock; the `資料:` line is what makes a missed fetch visible.

## Status: the run, not the data

`ok` and `failed` only — **never `degraded`**.

- **`ok`** — data was read and rendered, however stale, and even if a few
  series are missing. Staleness is a fact about the data, and reporting it as
  `degraded` would be a verdict about the data, which this skill does not make.
  It would also trip `repair_policy=retry_once`, and a retry cannot repair
  stale upstream data — it would just deliver the identical message twice.
- **`failed`, and nothing delivered** — the store is unreachable, the read
  fails, there are zero usable observations (empty store or every configured
  series missing), or a series lacks its `kind` so the family split cannot be
  rendered. This follows the repo's standing rule that a hard-failure path must
  not deliver.

"Seven days old" is `ok`. "Nothing to report at all" is `failed`.

## Config

The series list is a DB config value, not code:

```
cds_series = key|SERIES_ID|Label|kind;…
```

`kind` is `spread` or `yield`. Three-field rows (no `kind`) still parse, but a
series without one **fails the run** rather than being guessed — assuming
"yield" for an unlabelled spread is the false signal the split exists to
prevent.

## Not a CDS quote

Real CDX/iTraxx premiums are licensed terminal data and are not obtainable
here. What is stored is the free public proxy — bond option-adjusted spreads
and effective yields from FRED — and every row records
`source = "fred:<SERIES_ID>"` so it can never be mistaken for a CDS quote.
