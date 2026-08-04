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

The store holds all nine (`credit-store`'s eight configured series plus the
derived `baa−aaa`); the daily message narrows that to five — see **Message
layout** below.

## Message layout: what is data and what is code

Two readability passes happened the same day, 2026-08-04. v2 replaced the
one-row-per-series table with a vertical block per series, cut percentiles
down to bare counts, and split the message into a daily set and a monthly
set. It shipped, and the owner — the message's only reader — could not read
it. v3 replaced it hours later: the daily/monthly split is gone entirely,
the message shows five series instead of nine, and the count each window
prints carries its share again. Full rationale, the review trail, and what
each pass cost are in
`docs/specs/2026-08-04-cds-con-readability-v2-design.md` (its final `# v3`
section is the part that matters for today's code — v2's own body is
superseded); this section documents what the code actually does now.

**The message shows five series: `baa−aaa`, `baa10y`, `hy_oas`, `ig_oas`
(spreads) and `baa` (the one yield kept).** Which five, and in what order, is
a config value, `cds_message_series` — a comma-separated list of series keys
(mandatory; a missing key, a failed read, or an unparseable value each fail
the run loudly and by name, same as a missing `cds_series` or a missing
`kind`; see **Config** below). `cds_series` itself still carries all eight
series, and `price cds show`/the fetch job are unaffected — only the message
is narrowed.

**`baa` is kept on purpose, not left over.** It is the *same bonds* as
`baa10y` — one with the risk-free rate left in, one with it subtracted —
placed in separate blocks so the pair demonstrates the spread/yield
difference arithmetically rather than asserting it: the block header reads
「所以這條數字高,可能是央行升息,不是公司快倒閉」. A reviewer who read a high
yield alone as company-level stress, without seeing the matching spread sit
mid-range the same day, is exactly the misreading this pairing exists to
prevent.

**Series names come from the `Label` field of `cds_series`, not from Rust.**
Each series renders as a block: a title line `Label  value   [key]`, then one
line per trailing window. Translating or renaming a series is a config
change and never touches code. The one exception is the derived quality
spread, which is computed in `render.rs` and so is named there
(`BAA_AAA_LABEL`) — there is no config row to carry it. The key still rides
along in `[brackets]` on the title line: the reader and the operator are the
same person, and `[baa10y]` is what he types into `price cds show` and edits
in `cds_series`, so dropping it would force a lookup. The FRED series id
stays out — longer, and it cannot be passed to anything.

**There is no column alignment, and there will not be again.** The old
column machinery (`display_width`/`pad_to`/`RowWidths` from the table layout)
was deleted, not disabled: `run.rs` sets `parse_mode: None`, so Telegram
renders the body in a **proportional** font, where space padding never
produced a column for anyone.

**Every window prints a count with its share:
`{n} 筆裡 {below} 筆比現在低({share}%)`.** `近1年 250 筆裡 61 筆比現在低(24.4%)`.
The count states the whole before the part so magnitude reads without mental
division; `below` is exactly `credit-store`'s strictly-below comparison
(`values.iter().filter(|v| **v < x).count()`), so the wording is always
「低於」, never 「不高於」 (`wording_is_strictly_below_never_at_most`). The
share is truncated (never rounded) from that same `(below, n)` pair, never
computed separately, so it can never disagree with the count on the same
line (`share_percent_is_truncated_never_rounded_up`). A window sitting at
its minimum prints `0 筆裡 0 筆比現在低(0.0%)` — never a blank, an omitted
window, or a dash — which is what kills the old `p0` ambiguity, where a
truncated `p0` could mean either "the lowest value" or "0.9% of the window is
lower".

**A rate and a share must never appear on the same line — that is the rule
that survives, not "a percentile must never carry a `%` sign".** The earlier
wording is superseded: percentiles are gone, and both values and shares now
carry `%`. What the rule guards against is unchanged and is why it moved
rather than vanished: a value's `%` (a rate, e.g. `1.63%`) and a window's `%`
(a share of observations, e.g. `24.4%`) are two different meanings of one
symbol, and v3 keeps them apart by putting the value on the series' title
line and every share on its own window line below — never both on one line
(`rate_and_share_never_share_a_line`, the direct successor to v2's
`percent_marks_values_but_never_percentiles`).

**`全庫` is gone; the full-history window is labeled by its actual start
year.** `baa−aaa` covers 1919, `baa10y` 1986, the ICE/BAML series 2023 — three
different rulers that a fixed placeholder word hid. The window now reads
`自1919`/`自1986`/`自2023`, taken from the first stored observation's year,
never supplied or guessed (`window_label_is_the_actual_start_year`,
`three_series_with_different_coverage_get_three_different_labels`,
`start_year_label_is_derived_from_the_data_not_supplied`).

**The daily/monthly split is gone entirely.** v2 built it because the table
ran to 58 lines; v3 is 28. There is no `cds_monthly_expand_days`, no days-1–7
expansion rule, no collapsed monthly status line, and no "a wrong proxy stays
auditable" limitation — all of it, and the config key that drove it, were
deleted along with the mechanism, not merely hidden. It would also have
actively broken v3: `baa` is monthly, so under the old split the
spread-vs-yield contrast the message exists to show would have been absent
roughly 29 days a month. All configured series in `cds_message_series` render
every day, unconditionally.

**Line width is guaranteed by the renderer, not by config staying short.** A
series' title line (`Label  value   [key]`) is measured under a CJK-is-2
display-width model against a bound (`WIDTH_BOUND`, 48 columns); a line that
would exceed it **splits** — the label alone on its own line, then
`  value   [key]` indented like a window row — instead of silently growing
past it (`overlong_ascii_label_splits_the_title_line`,
`overlong_cjk_label_splits_the_title_line`). Short labels render
byte-identical to the unsplit form. The one case this cannot fix: a label
that alone already exceeds the bound still overflows on its own line — the
renderer never truncates a configured label to force a fit, since discarding
real `cds_series` data would be worse than one wrapped line. The transport
is still a **proportional** font, so this model is a coarse proxy against a
line bloating back to desktop-monospace-breaking size, not a guarantee
against wrapping on a phone.

**The footer is fixed prose, not a computed contrast.** v2's footer computed
a sentence naming the earliest- and latest-start series actually rendered
that day; it depended on the daily/monthly split to know which rulers were
on screen, and was deleted along with it. v3's footer states only what a
window is for: 「SIGNAL-ONLY:窗口越短對當下越敏感,越長越穩定。它們回答不同的
問題,不可跨列比。」

**The `SIGNAL-ONLY` marker stays on the footer.** It is a project-wide boundary
marker, not prose; only the explanation after it changed. Removing it during
a readability pass would be caught by the contract tests.

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
  rendered, `cds_message_series` is absent, unreadable, or unparseable (empty,
  or a blank key from a stray/doubled comma), or a `cds_message_series` key
  names a series that does not exist among the loaded series. This follows the
  repo's standing rule that a hard-failure path must not deliver.

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

Which series the daily message shows, and in what order, is a separate DB
config value, also not code:

```
cds_message_series = baa−aaa,baa10y,hy_oas,ig_oas,baa
```

A comma-separated list of series keys (the derived `baa−aaa` key is a valid
member). **This key is mandatory.** Absent, unreadable, or unparseable —
empty, or a blank token from a leading/trailing/doubled comma — each **fail
the run** rather than being guessed, same as a missing `cds_series` or a
missing `kind` above. A key that does not name any loaded series also fails
the run, by name, rather than being silently dropped. There is never a Rust
literal standing in for this list in the production path; `cds_series`
itself is unaffected and keeps carrying all eight source series regardless
of what the message shows.

## Not a CDS quote

Real CDX/iTraxx premiums are licensed terminal data and are not obtainable
here. What is stored is the free public proxy — bond option-adjusted spreads
and effective yields from FRED — and every row records
`source = "fred:<SERIES_ID>"` so it can never be mistaken for a CDS quote.
