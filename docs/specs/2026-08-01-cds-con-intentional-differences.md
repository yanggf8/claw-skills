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

## Known limits

- **Frequency is inferred, not declared.** `SeriesInput` needs a frequency and
  the config carries none, so it is derived from observation gaps: median ≥ 20
  days is monthly. This sits uneasily beside the `kind` decision, which added a
  config field precisely because *inferring* the family was unsafe, and a
  misclassification here would silently place a series in the wrong group on
  the `資料:` line. Measured before accepting: the six daily series have gaps of
  1–5 days and the two monthly ones 28–31, so the threshold sits in a 23-day
  void with 5.6× margin. Accepted rather than forcing a fifth config field. It
  needs revisiting if a source ever supplies calendar-daily rows — the same
  class of assumption as `MIN_SPAN_DAYS` against `WINDOW_SIZE` in oilcon.
- **The `config` table is not in `credit-store`.** It lives in `price-cli`'s
  private store module, so `run.rs` embeds a one-line
  `SELECT value FROM config WHERE key = ?`. One trivial query is tolerable —
  unlike the percentile logic, which is deliberately single-sourced — but a
  third consumer should move the read into `credit-store`.
- **`price_registry()` is duplicated in `main.rs`**, because `price-cli` is a
  `[[bin]]`-only crate with no `[lib]` and cannot be imported.
- **The two cron jobs are ordered by the clock alone.** Fetch at 06:00, deliver
  at 06:30. Nothing enforces that the fetch succeeded first; the `資料:` line is
  what makes a missed fetch visible.
- **The plan's golden message and the live output differ in one tie.** `aaa`
  and `baa` both start 1919-01-01, so the ordering rule falls through to config
  order — and the live config lists `baa` first while the golden was written
  `aaa` first. Both are correct under the rule; the golden simply predates the
  live config's order.

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
