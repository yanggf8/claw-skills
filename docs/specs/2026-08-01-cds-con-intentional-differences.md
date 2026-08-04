# cds-con — design record and known limits

Not a port. There is no Python original, so this file records **decisions and
limits** rather than differences from an oracle. Anything not listed here that
surprises you is a bug.

Crate: `crates/cds-con`. Shared logic: `../gwebcdb/crates/credit-store`.
Live 2026-08-01; first scheduled runs 06:00 (fetch) and 06:30 (deliver) Taipei.

## The one that will surprise a reader: no `狀態：` line

Every other skill here opens with a classified status. cds-con does not, by
decision, and it must not be "fixed".

A percentile is a rank **within a stated window**, and the window flips the
conclusion. Measured 2026-08-01: `baa10y` sat at p26.0 over one year, p13.4
over ten, p14.3 over the store. Any ladder must pick a window and would then be
reporting that choice as much as the market. `price cds show` already emits no
verdict for the same reason, and the rule is not weakened by this message being
pushed rather than pulled — that was asked directly and answered.

## Two families, deliberately not comparable

| | Series | |
|---|---|---|
| **Spreads** | `baa−aaa`, `baa10y`, `hy_oas`, `ig_oas` | risk-free rate removed |
| **Yields** | `aaa`, `baa`, `hy_yield`, `ig_yield`, `ccc_yield` | risk-free rate included |

On 2026-08-01 the yields sat near one-year highs (`ig_yield` p97.7,
`ccc_yield` p98.5) while the spreads sat far lower (`baa10y` p13.4 over ten
years). **That is a fact about interest rates, not credit stress.** Separate
blocks with their meaning on the header are what stop a reader manufacturing a
signal by comparing across them.

**Coverage is mixed inside each family**, so it is printed per series line, not
per block: `aaa`/`baa` reach 1919, `baa10y` 1986, the five ICE/BAML series only
~3 years (a licensing cap on the keyless FRED endpoint, not a bug). An earlier
plan revision claimed all three Moody's series reached 1919 — `baa10y` starts
1986, and the figures contradicting the claim were in the same session's own
query output.

## Status reports the run, not the data

`ok` and `failed` only — **never `degraded`**.

- **`ok`, delivers** — data read and rendered, however stale, even with a few
  series missing.
- **`failed`, delivers nothing** — store unreachable, read fails, zero usable
  observations, or a series lacking `kind`.

Two reasons, both decisive. Reporting stale data as `degraded` would be a
verdict about the data, contradicting the decision above; and `degraded` trips
`repair_policy=retry_once`, which cannot repair stale upstream data and would
deliver the identical message twice. Meanwhile `ok` on an empty store would
have delivered an all-`n/a` report and left a dead pipeline permanently green,
because the scheduler reads markers and not the body — `CLAUDE.md:266-267`
forbids exactly that.

"Seven days old" is `ok`. "Nothing to report at all" is `failed`.

## Readability pass (2026-08-02) — what was traded and what was not

Reviewed by Grok against one question: what is hard to understand here for a
non-specialist. Three of its suggestions were **rejected**, and the reasons matter
more than the accepted ones:

- **Colloquial anchors** (「偏窄」「很高」 beside each value) — rejected. 「偏窄」 IS a
  verdict. `baa10y` is p26 over one year and p13 over ten; calling it 「偏窄」
  smuggles in a ladder whose window is never stated. Grok itself graded this the
  worst offender against the standing constraint.
- **A two-line summary at the top** — rejected, and for a harder reason than Grok
  gave. That text would be a fixed sentence about today's shape ("yields high,
  spreads low"). The day spreads actually widen it becomes false, with nothing to
  catch it. It is the hardcode problem wearing a prose costume.
- **Dropping any of: the two-block split, per-line coverage, all windows side by
  side, the freshness line** — rejected. These are where the honesty lives.
  Showing `品質利差` at p0/p0/p3 and `中級利差` at p26/p13/p14 is what makes "how low
  depends on the ruler" visible; collapsing to one window would hide it.

Accepted: Label instead of key, whole-number percentiles, `自1986 日` instead of
`1986-01-02→ daily`, a worked example in the footer, and dropping the `CDS-CON`
system name from the title (redundant for a single reader).

**Grok was explicit that (e) has no good answer** — there is no way to make
window-dependence intuitive to someone who does not know what a percentile is
*without* collapsing it into a verdict. Colour, arrows, a single window, and
"usually look at the 10-year" were all judged to be ladders in disguise. The
worked example is the least-bad option, and it demonstrates rather than teaches.

**Codex caught two labels that were wrong, verifiable from our own config.** The
proposed `CCC 殖利率` dropped "and lower" (BAMLH0A3HYCEY is CCC *and lower*), and
`中級利差 Baa−10年債` invented a generic name for what is specifically Moody's Baa
minus the 10-year **US Treasury** — both contradicted by the English labels the
config already carried. Corrected to `CCC 及以下殖利率` and `Baa−10年期美債利差`.

Codex also flagged that the five ICE BofA series are capped by FRED at the last
three years, so a coverage claim of `自1986` for one of them would be a lie. It
cannot happen: `coverage_start` is taken from `rows[0].date` of what is actually
stored, never from config. The ICE lines correctly read `自2023`.

**Truncation over rounding was found by running the tests, not by review.** The
golden regenerated with `ccc_yield` at `p100` (true value 99.6). That claims the
top of the window while 0.4% sits above it. Truncation understates by under one
percentile and is always true.

## Readability pass v2 (2026-08-04)

The first pass above fixed the *words*. It was judged improved but still not
enough, and a second, independent defect surfaced that the first pass never
looked at: the message was laid out as a table on a transport that cannot
render one. Full design, decisions, review trail, and the target-output
mocks are in `docs/specs/2026-08-04-cds-con-readability-v2-design.md` — this
entry records the three things a future reader would otherwise re-litigate.

**Alignment was abandoned, not fixed.** `run.rs` sets `parse_mode: None`, so
Telegram renders the body as plain text in a **proportional** font. Space
padding does not produce columns in a proportional font, so the entire column
machinery (`display_width`/`pad_to`/`RowWidths`, and the table-row layout they
served) was never reaching any reader on this transport — there was no
alignment to fix, only to delete. cds-con was the only sibling skill
(chipcon/inflation-con/oilcon compared) that built a table at all, and the
one on the transport that cannot show one.

**No share percentage sits beside a count.** Percentiles became counts
(`N/M 筆低於本次`) in this pass. A draft wrote `61/250 筆低於本次(24.4%)`; the
owner rejected the parenthetical on 2026-08-04. The reason is not
cosmetic: `(24.4%)` would sit one line under a value's own `1.63%`, and the
two are different meanings of the same symbol — a **rate** (the value) beside
a **share of observations** (the parenthetical) — collapsing exactly the
distinction SKILL.md's percent rule exists to keep apart. The count already
carries the definition (`61 of 250 observations are lower than this one` is
readable from `61/250` alone), so the parenthetical would have added a second
representation of the same fact with its own correctness obligation (staying
in sync with `below`/`n`) for no reader benefit.

**The days-1–7 monthly-expand rule is a proxy, and it is wrong in one specific,
named way.** There is no way to detect *when* FRED actually published a
monthly value — a monthly observation is always dated the 1st of the previous
month and always lands 30–35 days old, so the gap between the observation date
and `as_of` carries no transition signal, and cds-con writes nothing so it has
no memory of what it showed last time. The calendar-day bound
(`cds_monthly_expand_days`, default 7) is a deliberate proxy for that
undetectable event, and the honest claim about it is narrower than "nothing is
ever dropped": **if FRED publishes after day 7, that month's values stay
collapsed for the rest of the month, and the only thing that moves is the
date stamp** on the monthly status line
(`月頻 3 列 資料至 YYYY-MM,未展開(…)`). The safeguard's actual guarantee is
that the monthly series remain visible **as a group** and that how far behind
the data is can always be read — including naming a series that is missing
even while the block is collapsed (`・缺 <key>`) — not that a reader is
guaranteed to see that month's numbers. So the correct framing is **a wrong
proxy stays auditable**, never "nothing is ever silently dropped."

## Readability pass v3 (2026-08-04, same day) — v2 shipped and its only reader could not read it

v2 above was implemented, delivered to the phone, and the owner could not
understand it. That is the only verdict that matters for a message with one
reader: a message the reader cannot read has failed, regardless of how well
it satisfies its own constraints — v2 was optimised for defensibility over
legibility. v3 replaced it hours later. Full design, the owner's own words,
the review trail, and the target mock are in the `# v3` section of
`docs/specs/2026-08-04-cds-con-readability-v2-design.md`; this entry records
the three things a future reader would otherwise re-litigate.

**The daily/monthly split was built, reviewed, merged, and deleted within a
day — and that is not waste.** It was sized correctly for the message it was
built for: v2 ran to 58 lines, and the split was what kept 41 of them off the
screen on an ordinary day. Cutting the message to five series (v3 §1) made
it 28 lines, well under the length the split existed to manage, and the split
then became a straight liability: `baa` — one of the five kept series — is
monthly, so under the old split the spread-vs-yield contrast the whole
message exists to demonstrate would have been missing about 29 days a month.
Deleted with it: `expand_days` on `format_message`/`render_lines`,
`monthly_expand_days`, the `cds_monthly_expand_days` config key, the
collapsed monthly status line, the missing-monthly-series safeguard,
`day_of_month`, `expand_monthly`, and the "days-1–7 is a proxy that can be
wrong" limitation two sections above. Record this so nobody rebuilds it from
scratch next time the message grows: **a mechanism for managing length is a
liability once the message is short enough not to need one** — check the
line count before reaching for a collapse strategy, not after.

**The `%` ban was reversed on evidence, not on taste.** v2 §2 of the design
doc dropped the share percentage from each window (`61/250 筆低於本次`, no
`(24.4%)`), reasoning that the count alone carried the definition and a
parenthetical share would collide two meanings of `%` on adjacent lines. That
reasoning was internally consistent and shipped anyway broken: the owner
could not read `61/250` without doing the division himself, so the message
failed on the one axis that matters — comprehension by its one reader. v3
restored the share, and fixed the collision by moving the *value* off its own
line and onto the series' title line instead (v3 §3), so a value's `%` and a
window's `%` never sit on adjacent lines to begin with. The lesson: dropping
a symbol to avoid an ambiguity is only a fix if the reader can still get the
information some other way; here the "other way" (mental division) does not
exist for the reader this message has.

## Known limits

- **Frequency is inferred, not declared.** `SeriesInput` needs a frequency and
  the config carries none, so it is derived from observation gaps: median ≥ 20
  days is monthly. This sits uneasily beside the `kind` decision, which added a
  config field precisely because *inferring* the family was unsafe.
  **Corrected 2026-08-04 (v3): the blast radius this note previously described
  no longer applies.** Between the two passes that day, frequency's role
  changed twice. Written when frequency was only a display word on the `資料:`
  line, this note said a misclassification "would silently place a series in
  the wrong group" there. Readability v2's daily/monthly split then made
  frequency the switch deciding **whether the series rendered at all** for
  ~29 days a month — a much larger blast radius, recorded in this note's
  earlier revision. v3 deleted that split hours later (see below), and with it
  the only path by which a frequency misclassification could hide a series.
  Frequency's entire remaining job is grouping a series' latest date into the
  `日 至` or `月 至` half of the freshness line
  (`format_freshness_line`/`min_latest`, `render.rs:575-577`) and choosing the
  header date's fallback (`header_date`, `render.rs:406`) — a misclassified
  series still always renders its full block, value and windows included; a
  wrong inference only misfiles its date into the wrong freshness bucket, or
  makes the header date fall back one step further than it should. Measured
  before accepting: the six daily series have gaps of 1–5 days and the two
  monthly ones 28–31, so the threshold sits in a 23-day void with 5.6×
  margin. Accepted rather than forcing a fifth config field. It needs
  revisiting if a source ever supplies calendar-daily rows — the same class of
  assumption as `MIN_SPAN_DAYS` against `WINDOW_SIZE` in oilcon.
- **The `config` table is not in `credit-store`.** It lives in `price-cli`'s
  private store module, so `run.rs` embeds a one-line
  `SELECT value FROM config WHERE key = ?`. One trivial query is tolerable —
  unlike the percentile logic, which is deliberately single-sourced — but a
  third consumer should move the read into `credit-store`.
- **`price_registry()` is duplicated in `main.rs`**, because `price-cli` is a
  `[[bin]]`-only crate with no `[lib]` and cannot be imported. The credential path
  itself was confirmed rather than assumed: `turso_util::resolve_token` checks the
  environment first (`lib.rs:30`) and returns immediately on a hit, the gateway
  process carries both `PRICE_TURSO_URL` and `PRICE_TURSO_READ_TOKEN`, and that
  token has no `exp` claim. So a cron host never mints and never needs an
  interactive `turso auth login`.
- **The two cron jobs are ordered by the clock alone.** Fetch at 06:00, deliver
  at 06:30. Nothing enforces that the fetch succeeded first; the `資料:` line is
  what makes a missed fetch visible.
- **The plan's golden message and the live output differ in one tie —
  moot under v3.** `aaa` and `baa` both start 1919-01-01, so `analyze()`'s
  ordering rule falls through to config order — and the live config lists
  `baa` first while an earlier golden was written `aaa` first. Both were
  correct under the rule; the golden simply predated the live config's order.
  This applied to v2, whose message rendered `analyze()`'s coverage-first sort
  directly. It no longer can under v3: message order instead follows
  `cds_message_series` literally (`select_message_series`, "the config *is*
  the display order"), and the configured five omit `aaa` entirely, so the tie
  cannot surface in the delivered message.

## Not a CDS quote

Real CDX/iTraxx premiums are licensed terminal data and unobtainable here. The
store holds the free FRED proxy and every row records
`source = "fred:<SERIES_ID>"` so it can never be mistaken for one.

## Verification at cutover

- `credit-store` extraction proven by capturing `price cds show` **before** the
  move (00:21:49, before the crate existed at 00:24:46) and byte-comparing
  after: all three captures hash to `ab7084e3a84bef3726ec9bfde20bc66a`. Merely
  rerunning the moved tests would not have been a differential — after the move
  both the CLI and its tests call the same implementation.
- `baa−aaa` figures verified with independent SQL, not taken from the
  implementation's report: n=1290, 1919-01-01→2026-06-01, latest 0.48,
  strictly-below full-history percentile 3.7209. Computed inclusively it is
  3.876, which rounds to p3.9 — the number an earlier plan revision carried.
- Cold-start fetch: 16.3 s for all eight series against a 300 s timeout, moving
  the daily series from 2026-07-24 to 2026-07-30.
- Smoke test through the path nullclaw resolves, with a job id: exit 0 in 2.0 s,
  markers in order, bare job id, clean stderr.
- Binary published by `tools/install-skill.sh cds-con`, whose smoke probe
  requires exit 2 on an unknown flag — the gate that caught the Phase ③
  argument-parsing defect on 2026-07-31.

## Rollback

Pause or remove the two cron jobs. Nothing else is affected: cds-con only
reads. The `cds_series` config now carries a fourth field, which old
three-field parsers still accept.
