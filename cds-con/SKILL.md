---
name: cds-con
description: Report corporate credit-spread levels and how many stored observations sit below each, within stated windows; signal-only, deliberately unclassified (no status ladder, no cheap-or-expensive label).
always: true
---

# cds-con

Report **corporate borrowing cost** — what a bond buyer is paid for taking
credit risk — as levels and counts (how many stored observations, in a stated
window, sit below today's value). This is **observation-only**: no entry/exit
advice, no position sizing, no portfolio edits.

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

## Message layout: what is data and what is code

Readability pass v2 (2026-08-04) replaced the one-row-per-series table with a
vertical block per series, replaced percentiles with counts, and split the
message into a daily set (shown every day) and a monthly set (shown only on
the first few days of the month). Full rationale, what it cost, and the review
trail are in
`docs/specs/2026-08-04-cds-con-readability-v2-design.md`; this section
documents what the code actually does.

**Series names come from the `Label` field of `cds_series`, not from Rust.**
Each series renders as a block: a title line `Label [key]`, then a
`value  頻率・自YYYY` line, then one line per trailing window. Translating or
renaming a series is a config change and never touches code. The one
exception is the derived quality spread, which is computed in `render.rs` and
so is named there (`BAA_AAA_LABEL`) — there is no config row to carry it. The
key still rides along in `[brackets]` on the title line: the reader and the
operator are the same person, and `[baa10y]` is what he types into
`price cds show` and edits in `cds_series`, so dropping it would force a
lookup. The FRED series id stays out — longer, and it cannot be passed to
anything.

**There is no column alignment, and there will not be again.** The column
machinery (`display_width`/`pad_to`/`RowWidths`) was deleted, not disabled:
`run.rs` sets `parse_mode: None`, so Telegram renders the body in a
**proportional** font, where space padding never produced a column for
anyone. `every_rendered_line_fits_its_width_bound` is what replaced the old
`cjk_labels_keep_columns_aligned` alignment pin — it is a coarse proxy against
a line bloating back to a size that breaks even a monospace reader, not a
guarantee against wrapping on a phone.

**Percentiles are gone; every window prints a count instead:
`label  below/n 筆低於本次`.** `近1年 p24` became `近1年 61/250 筆低於本次`. This
is arithmetic, not an interpretation: `below` is exactly
`credit-store`'s strictly-below comparison (`values.iter().filter(|v| **v <
x).count()`), so the wording is always 「低於」, never 「不高於」
(`wording_is_strictly_below_never_at_most`), and what prints is that raw
`(below, n)` pair carried straight through, never a count reconstructed from
a rounded or scaled percentage
(`printed_count_is_the_raw_comparison_never_derived`). A window sitting at
its minimum prints `0/N` — never a blank, an omitted window, or a dash — which
is what kills the old `p0` ambiguity (a truncated `p0` could mean "the
lowest value" or "0.9% of the window is lower"; a count cannot)
(`zero_below_renders_as_zero_over_n`). **No share percentage rides beside a
count** — `61/250 筆低於本次(24.4%)` was drafted and rejected: a `%` one line
under a value's own `%` collides two meanings of one symbol, and the count
already carries the definition (`no_share_percentage_is_printed_beside_a_count`).
Values still carry `%` (OAS is commonly quoted in basis points, so a bare
`2.84` is ambiguous); the invariant that survives from the old percentile-era
rule is narrower now but still tested: **a percentile figure must never carry
a `%` sign** (`percent_marks_values_but_never_percentiles`).

**`全庫` is gone; the full-history window is labeled by its actual start
year.** `baa−aaa` covers 1919, `baa10y` 1986, the ICE/BAML series 2023 — three
different rulers that a fixed placeholder word hid. The window now reads
`自1919`/`自1986`/`自2023`, taken from the first stored observation's year,
never supplied or guessed (`window_label_is_the_actual_start_year`,
`three_series_with_different_coverage_get_three_different_labels`,
`start_year_label_is_derived_from_the_data_not_supplied`).

**Six daily series render every day; three monthly series
(`baa−aaa`/`baa`/`aaa`) render only on the first few days of the month.** The
split is by publication frequency, not by value, so the rule is identical
whichever way the market moves. The day bound is a config value,
`cds_monthly_expand_days` (mandatory — an absent key, a failed read, or an
unparseable value each fail the run loudly and by name, same as a missing
`cds_series` or a missing `kind`; see **Config** below), read fresh on every
run and evaluated against the injected `as_of` calendar date, never a wall clock
(`monthly_block_expands_only_within_the_configured_day_bound`,
`day_bound_is_evaluated_from_as_of_never_a_clock`). `as_of` is always the CST
date `main.rs` computes (`cst_today()`, fixed offset +8), so the bound is CST
by construction. On a collapsed day the footer carries a status line instead
of the block:

```
月頻 3 列 資料至 2026-06,未展開(每月 1–7 日展開)
```

and, if a monthly series has no value even while collapsed, an added
`・缺 aaa` — the ~29 days a month the block is hidden must not also hide that
a series is missing (`monthly_status_line_present_whenever_the_block_is_collapsed`,
`monthly_status_line_names_missing_monthly_series`). On an expanded day the
status line disappears and the monthly latest month is appended to the
`資料:` line instead (`・月 至 2026-06`) — the two are mutually exclusive
(`monthly_status_line_absent_when_expanded`). The derived `baa−aaa` row is
built from the full `baa`/`aaa` inputs *before* this filter is applied, so a
collapsed day still shows the correct quality spread on the days it is
shown — filtering earlier would silently stop it being derived at all.

**The footer contrast is computed from what actually rendered today, not
written as a fixed sentence.** `SIGNAL-ONLY:每個窗口各自回答自己的問題,不可跨列
比較——` is followed by one computed sentence naming the earliest-start and
latest-start series among the lines actually shown, e.g.

```
自2023 的 750 筆和自1986 的 10000 筆不是同一把尺。
```

Drawing the contrast from the rendered set (not the full configured set)
matters because a monthly series is collapsed on most days: an earlier draft
named `自1919` in the footer while `baa−aaa` sat hidden in the collapsed
block, pointing the reader at a ruler that was not on screen
(`footer_contrast_uses_only_series_rendered_today`). There is no separate
worked example anymore — the old `window_example` function, which picked one
line's two disagreeing windows to demonstrate window-dependence, was deleted
outright. With a count on every window of every line, the demonstration is
already on every line; a second one added nothing it didn't already show.

**The `SIGNAL-ONLY` marker stays on the footer.** It is a project-wide boundary
marker, not prose; only the explanation after it became concrete. Removing it
during the readability pass was caught by the contract tests.

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
  series missing), a series lacks its `kind` so the family split cannot be
  rendered, or `cds_monthly_expand_days` is absent, unreadable, or unparseable
  so the daily/monthly split has no bound to evaluate. This follows the repo's
  standing rule that a hard-failure path must not deliver.

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

The monthly-block day bound is also a DB config value, not code:

```
cds_monthly_expand_days = 7
```

Days 1 through this value (inclusive) show the monthly block; later days
collapse it to the status line described above. **This key is mandatory.**
Absent, unreadable, or unparseable each **fail the run** rather than being
guessed — same as a missing `cds_series` or a missing `kind` above — so there
is never a Rust literal standing in for it in the production path.

## Not a CDS quote

Real CDX/iTraxx premiums are licensed terminal data and are not obtainable
here. What is stored is the free public proxy — bond option-adjusted spreads
and effective yields from FRED — and every row records
`source = "fred:<SERIES_ID>"` so it can never be mistaken for a CDS quote.
