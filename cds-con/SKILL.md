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

## What it reads

The monthly **`baa`** series from `price-registry.credit_spreads` (1919-01
onward), written by `price cds fetch`. Nothing else reaches the message.

The series is **fixed in code, not configured**. Attribute 2 is defined on the
Baa yield, so which series answers it is a property of the research — a config
key able to redirect it silently is the exact shape of drift that let this
attribute measure the wrong thing for as long as it did. `cds_series` still
supplies that series' FRED id and display label.

`cds_message_series` and `cds_message_lead` are **no longer read**. Both rows
still sit in the registry and describe the retired spread message; they are
inert.

## Message layout

```
💾 企業債成本｜attribute 2

Baa 公司債殖利率  6.19%  07-01
狀態：高（as-of 分位 56，n=1291）

量的是殖利率本身,不是任何相減後的利差
分位只用該月之前(含當月)的觀測,不回望未來
切點是中位數,分位 ≥ 50 為「高」

──── 佐證 ────

  近1年 13 筆裡 12 筆比這一筆低(92.3%)
  近10年 121 筆裡 77 筆比這一筆低(63.6%)
  自1919 1291 筆裡 731 筆比這一筆低(56.6%)

這幾個窗口不是判定的依據,判定只看上面那個 as-of 分位

資料:月 至 2026-07
finance-cli `cost level` 是這條規則的權威,本訊息跟隨它。
```

**One label, and it names its basis.** The `狀態：` line always carries the
as-of percentile and its `n` — the same number reads differently on a window
of 129 and one of 1291, and hiding the base would be reporting a choice as if
it were a measurement.

**The trailing windows carry no label of their own.** They are a different
basis from the as-of reading, and labelling each would put several verdicts on
one number — the objection that used to rule out labelling anything at all.
They are evidence for the reader, not a second opinion, and the line under
them says so.

The 佐證 block is wrapped in `<blockquote expandable>` (Telegram HTML
`parse_mode`) so it collapses behind a tap. The reading and its basis stay
outside it: a verdict behind a tap is a verdict the reader never sees.

**A month the series does not reach prints `狀態：無資料`**, not a zero
percentile. Those are opposite findings — the cheapest borrowing on record
versus no observation at all — and `cost level` prints the same row rather
than dropping the line, because a missing line looks like a query that was
never run.

The retired spread message (the `baa10y`/`baa` lead pair, the 佐證 series
blocks, and the four readability passes behind them) is documented in
`docs/specs/2026-08-04-cds-con-readability-v2-design.md`. Its renderer is
still in `render.rs` below `render_parts`, with no caller.


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

`baa` **must be present** in `cds_series`: it is the series attribute 2 is
defined on, and the run fails by name without it. Which series the message
reports is not configurable — see **What it reads** above.

`cds_message_series` and `cds_message_lead` are inert. They configured the
retired spread message and are no longer read; the run does not fail if they
are absent, unparseable, or name series that do not exist. Delete them
whenever convenient.

## Not a CDS quote

Real CDX/iTraxx premiums are licensed terminal data and are not obtainable
here. What is stored is the free public proxy — bond option-adjusted spreads
and effective yields from FRED — and every row records
`source = "fred:<SERIES_ID>"` so it can never be mistaken for a CDS quote.
