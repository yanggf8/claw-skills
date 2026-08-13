---
name: cds-con
description: Daily push for attribute 2 of the finance-engineering research — the Baa corporate bond yield's own level, its as-of percentile, and whether borrowing reads 高 or 不高.
always: true
---

# cds-con

**This skill is not a standalone work.** It is the daily push for **attribute
2** of the `~/b/finance-engineering` research — "was corporate bond cost high
or not" — and it exists to carry that project's reading, in that project's
terms. Where this skill and finance-engineering disagree, finance-engineering
is right and this skill is the thing that changes.

## The authority is `finance-cli`, and the measure is the Baa yield itself

The owner's ruling of 2026-08-12 retired the charter and made **`cost level`
the single definition of attribute 2**: `attribute2()` in `finance-cli`
delegates to `cost_cmd::level_at`, so there is exactly one implementation of
the rule and changing the command changes the typing.

**What is measured is the Baa corporate bond yield itself. Nothing is
subtracted.** The implementation had drifted into measuring the *direction* of
the Baa−Aaa quality spread, which answers a different question — how much more
junk costs than quality, i.e. credit stratification, not whether borrowing was
expensive. A high-cost year can be compressing and a low-cost year widening.
Direction also could not separate the anchors: five of six carried the same
label, and 1966 (50th percentile) sat in the same class as 1999 (12th).

`crates/cds-con/src/cost.rs` mirrors `cost_cmd::level_at` and must stay
arithmetically identical to it, down to the integer truncation that decides
the label on the boundary.

## The reading

- **Basis: an as-of expanding window.** Only observations up to and including
  the one being read, never the future. This is what lets a reading be checked
  against what was knowable at the time.
- **`n` travels with every reading.** Early windows are thin (1929-09 rests on
  129 observations, 2026-07 on 1291). Thinness is a fact about the reading,
  not a blemish to hide.
- **The cut is the median**: an as-of percentile ≥ 50 reads `高`, below it
  `不高`. Owner's ruling, 2026-08-12.
- **A date the series does not carry prints 無資料**, and does not vanish. A
  dropped line looks like a query that was never run, and no-data is itself a
  finding this research records.

The series is the **monthly `baa`**, 1919-01 onward, from
`price-registry.credit_spreads`. Not the daily `DBAA`, which only reaches 1986
— attribute 2's anchors run back to 1929 and a 1986 series cannot read them.

## Recall-only

It reports a reading. It never recommends, predicts, ranks, or writes
anything back.

The message lays out levels, counts and coverage. The reader judges.

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

Two families that are **not commensurable**:

- **Spreads** (`baa−aaa`, `baa10y`, `hy_oas`, `ig_oas`) — compensation for
  credit risk, with the risk-free rate already removed.
- **Yields** (`aaa`, `baa`, `hy_yield`, `ig_yield`, `ccc_yield`) — total
  borrowing cost, which **includes** the risk-free rate.

On 2026-07-31 the yields sat near one-year highs while the spreads sat near
multi-decade lows. That is a fact about interest rates, not about credit
stress, and comparing a yield percentile against a spread percentile
manufactures a signal that is not there. This is why the two families never
mix anywhere in the message **except** the opening lead pair, which is
allowed to place one spread and one yield side by side ONLY because they are
the same underlying bonds and the message states, in the line right after
them, why they differ. See **Message layout** below for exactly where that
line is drawn.

**Coverage is mixed inside each family**, so it is printed on every series line
rather than a block header: `aaa`/`baa` reach 1919, `baa10y` 1986, and the five
ICE/BAML series only ~3 years — a licensing cap on the keyless FRED endpoint,
not a bug. `baa−aaa` is derived at read time from `baa` and `aaa`; it is the
longest-history credit-stress measure available.

The store holds all nine (`credit-store`'s eight configured series plus the
derived `baa−aaa`); the daily message narrows that to five — see **Message
layout** below.

## Message layout: what is data and what is code

Four readability passes have happened, the last three on the same day,
2026-08-04. v2 replaced the one-row-per-series table with a vertical block
per series, cut percentiles down to bare counts, and split the message into
a daily set and a monthly set. It shipped, and the owner — the message's
only reader — could not read it. v3 replaced it hours later: the
daily/monthly split gone entirely, five series instead of nine, counts with
their share restored. **The lead-block redesign, same day, went further
still**: the owner could read v3, but named the one sentence that actually
clicked — "利差在數十年低點、殖利率在一年高點" — a *contrast between two
numbers*, not any single series' block. The message now opens with exactly
that pair before falling back to the older per-series shape for everything
else. **A same-day lead-fix pass then caught two real defects a review
found in that lead block**: the closing explanation had drifted into a
verdict (`所以下面那條高…` asserted today's level outright, failing on a day
the yield falls) and an unsupported same-day arithmetic claim, and the title
line's single shared date silently implied one snapshot the series shown do
not actually share. Both are fixed — see the two paragraphs below for what
changed and why. Full rationale, the review trail, and what each pass cost
are in `docs/specs/2026-08-04-cds-con-readability-v2-design.md` (its `# v3`
and `# v4` sections) and `.superpowers/sdd/2026-08-04-cds-con-readability-v2/`
(the lead-fix report); this section documents what the code actually does
now.

**The message opens with a guided pair, then a `──── 佐證 ────` block for
everything else.** `cds_message_lead` names two of `cds_message_series`'
keys, in order — today `baa10y` (spread, "扣掉利率(利差)") and `baa` (yield,
"沒扣(總殖利率)"), the *same Baa bonds* with and without the risk-free rate.
Everything `cds_message_series` names that is not in the lead renders below,
in config order, under the 佐證 heading.

```
扣掉利率(利差)  1.63%  07-31
  近1年 24.4%  近10年 12.6%  自1986 13.7%

沒扣(總殖利率)  6.19%  07-01
  近1年 92.3%  近10年 95.8%  自1919 50.5%

上面那條的算法,就是下面那條減掉十年期美債(同一天的)
但兩排的百分比不能相減 —— 排名不是水位

──── 佐證 ────

Baa 比 Aaa 多出的殖利率  0.43%  07-01
  近1年 13 筆裡 0 筆比這一筆低
  ...
```

Every title line carries the series' own `MM-DD` latest date (the header no
longer carries one shared date — see **The header has no date; every series
states its own** below).

**Putting a spread and a yield adjacent is normally forbidden, and is safe
here for exactly one reason.** The two families are not commensurable (a
percentile-style comparison across them manufactures a signal that is not
there — that is the whole reason v1/v2/v3 rendered them in separate blocks).
The lead pair is the ONE place this repo allows a spread and a yield to sit
next to each other, and only because (a) it is the *same underlying bonds*
and (b) the explanation lines right after them say how they relate and warn
against the one comparison that still manufactures a signal:
`上面那條的算法,就是下面那條減掉十年期美債(同一天的)` (how the top line is computed —
never a claim about which line is higher today) and `但兩排的百分比不能相減
—— 排名不是水位` (a percentile is a rank within a window, not a level; unlike
the two dollar-levels above it, it cannot be subtracted between the rows).
`require_single_kind` in `render.rs` enforces the other half of that
guarantee programmatically: the 佐證 block (whatever is left over) has no
per-kind header anymore, so `format_message` **fails the run** if it ever
ends up holding both a spread and a yield — an accidental config mix would
otherwise silently reopen the exact unexplained adjacency the old split
existed to prevent.

**The header has no date; every series states its own.** An earlier version
put one shared date on the `💾 信用利差` header line, borrowed from the
series shown. A 2026-08-04 review caught that as false: `cds_message_lead`
pairs a daily series with a monthly one by design, and even two daily series
from different providers can post their latest observation a day apart — a
single header date silently implied one snapshot that was not true. The
header is now bare, and every title line (lead and 佐證 alike) carries its
own `MM-DD` suffix instead, taken from that series' own `latest` date. A
series with no data still renders `n/a`, with no date (there is no
observation to date).

**The lead block trades the count for a bare share, compressed onto one
line.** `{override_label}  {value}%  {MM-DD}`, then `  {window} {share}%
{window} {share}%  ...` — every window the series supports, on a single
line, no `筆裡`/`筆比這一筆低` wording and no `[key]` (the lead reader is
comparing two numbers, not looking anything up). This is a deliberate trade
against the 佐證 block below: the lead needs the whole pair to fit in four
short lines, so it gives up the count's "magnitude without mental division"
that the 佐證 block still has.

**Lead labels are prose about the PAIRING, from `cds_message_lead`, never
from `cds_series`' own `Label`.** `扣掉利率(利差)` and `沒扣(總殖利率)` describe
how the two lines relate to each other, which is the wrong job for a
series' general-purpose label (`cds_series`' `Baa 比 10年期美債多出的殖利率`
is accurate but far too long, and says nothing about the *other* line in the
pair). See **Config** below for the `cds_message_lead` format.

**The 佐證 block keeps the older shape: `Label  value  MM-DD`, then one line
per window with the full count.** No `[key]` here either — the operator-lookup
argument that once justified `[key]` on every title line no longer holds now
that the lead (this message's headline content) never carried one, so it was
dropped from 佐證 too for one consistent title shape. Series names still come
from the `Label` field of `cds_series`, not from Rust; translating or
renaming a series is a config change and never touches code. The one
exception is the derived quality spread, computed in `render.rs` and so
named there (`BAA_AAA_LABEL`) — there is no config row to carry it.

**There is no column alignment, and there will not be again.** The old
column machinery (`display_width`/`pad_to`/`RowWidths` from the table layout)
was deleted, not disabled: Telegram renders the body in a **proportional**
font, where space padding never produced a column for anyone. This stayed
true when `parse_mode` became `HTML` — HTML is not `<pre>`, so the font is
still proportional and the machinery would still be useless.

## Delivery is HTML, and that makes escaping load-bearing

**`run.rs` sets `parse_mode: Some("HTML")`** (it was `None` until 2026-08-05).
The reason is the 佐證 block: Telegram collapses
`<blockquote expandable>…</blockquote>` behind a tap, which keeps the lead
pair and the freshness line visible while the evidence folds away. Nothing
else is marked up — no bold, no italics. `oilcon` already used a non-`None`
parse_mode, so this is a per-skill choice and breaks no shared invariant.

**The message has two representations and they are not interchangeable:**

- **stdout stays plain text with zero markup** — that is what a human or an
  agent reads when the tool runs without `--deliver-to`, and it is also what
  gets dumped on a delivery failure, where the point is to salvage readable
  content.
- **the Telegram payload is the HTML variant**, built by `flatten_html`.

**⚠ Escaping is now a delivery-breaking constraint, not a nicety.** Series
labels come from the `cds_series` DB config, which the owner edits. Before
this change a strange label made a line wrap; now **an unescaped `<` or `&`
in a label makes Telegram reject the whole message and the morning's report
never arrives.** So every piece of text that did not originate as literal
markup in this crate is escaped (`&`, `<`, `>`) *before* any tag is wrapped
around it — labels, keys, dates, values, and the appended job id. This
mirrors `plan-viewer-rs`, which escapes every author-supplied field and
asserts it with complete per-field coverage rather than a spot check.

**Known limit:** escaping is enforced because every send flows through
`render_message_parts` → `flatten_html`. A future second send path would not
inherit it automatically — that is a convention held by tests, not by the
type system.

**Every 佐證 window prints a count, with its share when something sits below
it: `{n} 筆裡 {below} 筆比這一筆低({share}%)`.** `近1年 250 筆裡 61 筆比這一筆低(24.4%)`.
The count states the whole before the part so magnitude reads without mental
division; `below` is exactly `credit-store`'s strictly-below comparison
(`values.iter().filter(|v| **v < x).count()`), so the wording is always
「低於」, never 「不高於」 (`wording_is_strictly_below_never_at_most`). The
share is truncated (never rounded) from that same `(below, n)` pair, never
computed separately, so it can never disagree with the count on the same
line (`share_percent_is_truncated_never_rounded_up`). **A window sitting at
its minimum (`below == 0`) drops the parenthetical entirely and prints just
`0 筆裡 0 筆比這一筆低`** — the lead-block redesign's one asymmetry: a bare
`0.0%` cannot tell a reader "exactly zero" from "truncated down from
something small", but the count sitting right there can. The lead block, by
contrast, always prints the share (including `0.0%`) since it has no count
to fall back on.

**A rate and a share must never appear on the same line.** A value's `%`
(a rate, e.g. `1.63%`) and a window's `%` (a share of observations, e.g.
`24.4%`) are two different meanings of one symbol. In the 佐證 block this
still means at most one `%` per line — the value sits on the title line,
every share on its own window line below. The lead block's compressed
windows line legitimately carries several `%` on one line, but they are
always all shares; the lead's own rate never joins them, because it lives on
a separate title line one row up.

**`全庫` is gone; the full-history window is labeled by its actual start
year.** `baa−aaa` covers 1919, `baa10y` 1986, the ICE/BAML series 2023 — three
different rulers that a fixed placeholder word hid. The window now reads
`自1919`/`自1986`/`自2023`, taken from the first stored observation's year,
never supplied or guessed (`window_label_is_the_actual_start_year`,
`three_series_with_different_coverage_get_three_different_labels`,
`start_year_label_is_derived_from_the_data_not_supplied`).

**The daily/monthly split is gone entirely (removed in v3, unchanged since).**
There is no `cds_monthly_expand_days`, no days-1–7 expansion rule, no
collapsed monthly status line. All configured series in `cds_message_series`
render every day, unconditionally.

**Line width is guaranteed by the renderer, not by config staying short.** A
title line (`Label  value`, in either block) is measured under a CJK-is-2
display-width model against a bound (`WIDTH_BOUND`, 48 columns, unchanged by
the lead-block redesign — the widest line the new shape produces, the lead's
compressed windows line, measures 41); a line that would exceed it
**splits** — the label alone on its own line, then `  value` indented like a
window row — instead of silently growing past it
(`overlong_ascii_label_splits_the_title_line`,
`overlong_cjk_label_splits_the_title_line`). Short labels render
byte-identical to the unsplit form. The one case this cannot fix: a label
that alone already exceeds the bound still overflows on its own line — the
renderer never truncates a configured label to force a fit, since discarding
real config data would be worse than one wrapped line. The transport is
still a **proportional** font, so this model is a coarse proxy against a
line bloating back to desktop-monospace-breaking size, not a guarantee
against wrapping on a phone.

**The footer is fixed prose, not a computed contrast.** 「SIGNAL-ONLY:窗口越
短對當下越敏感,越長越穩定。」

**The `SIGNAL-ONLY` marker stays on the footer.** It is a project-wide boundary
marker, not prose. Removing it during a readability pass would be caught by
the contract tests.

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
  or a blank key from a stray/doubled comma), a `cds_message_series` key
  names a series that does not exist among the loaded series,
  `cds_message_lead` is absent, unreadable, or unparseable (not exactly two
  `key|Label` records), a `cds_message_lead` key names a series not present
  in `cds_message_series`, or the 佐證 block ends up holding both a spread
  and a yield (see **Message layout** above). This follows the repo's
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

Which two of those keys open the message as the guided pair, and what each
is called there, is a third DB config value:

```
cds_message_lead = baa10y|扣掉利率(利差);baa|沒扣(總殖利率)
```

`key|Label` records joined by `;`, **exactly two** — it is a pair by
construction, not an open-ended list. Each `Label` is prose about the
*pairing* (see **Message layout** above), never `cds_series`' own per-series
label. Same no-default standard as `cds_message_series`: absent, unreadable,
unparseable (wrong field count, an empty field, a duplicate key, or not
exactly two records), or a key that does not name a series present in
`cds_message_series`, all **fail the run** rather than being guessed. There
is no requirement that the two entries be one spread and one yield — that
correctness is the operator's responsibility, the same way a wrong
`cds_series.Label` is; what Rust *does* enforce is that whatever is left
over in the 佐證 block (everything `cds_message_series` names outside the
lead pair) never mixes spreads and yields, since that block has no per-kind
header to keep them apart.

## Not a CDS quote

Real CDX/iTraxx premiums are licensed terminal data and are not obtainable
here. What is stored is the free public proxy — bond option-adjusted spreads
and effective yields from FRED — and every row records
`source = "fred:<SERIES_ID>"` so it can never be mistaken for a CDS quote.
