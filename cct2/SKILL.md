---
name: cct2
description: Dual-LLM market sentiment analysis for configured tickers — pre-market and EOD reports delivered to Telegram
always: true
---

# cct2

Fetches prices and headlines from Yahoo Finance, runs sentiment analysis in parallel with two LLMs (primary + backup), and delivers a report to Telegram. Flags tickers where the two models disagree.

When running under cron, the `NULLCLAW_JOB_ID` is appended to the Telegram message and prefixed on every log line. After a successful delivery the script also emits scheduler verification markers on stdout: `[skill-status:ok]` plus `[trace:<job_id>]` on separate lines. If the script finishes but produces no analyzable results, it emits `[skill-status:failed]` plus the trace marker so cron can retry and alert correctly.

## Script

```
~/.nullclaw/skills/cct2/bin/cct2
```

## Usage

```
~/.nullclaw/skills/cct2/bin/cct2 --mode pre-market
~/.nullclaw/skills/cct2/bin/cct2 --mode eod
~/.nullclaw/skills/cct2/bin/cct2 --mode pre-market --deliver-to 7972814626
~/.nullclaw/skills/cct2/bin/cct2 --mode eod --deliver-to 7972814626 --account main
```

## Options

- `--mode MODE` — pre-market | eod (required)
- `--deliver-to CHAT_ID` — Send output to Telegram chat instead of stdout
- `--account ACCOUNT` — Telegram account name (default: main)
- `--et-hour H` — run only when the US-Eastern hour is `H`; otherwise exit `ok` without delivering (DST gate)

## Tickers

Stored in nullclaw memory under key `cct2:tickers` (category: skill).
Default if not set: AAPL MSFT GOOGL TSLA NVDA

Update via agent:
> Remember that cct2 tickers are AAPL MSFT GOOGL TSLA NVDA AMD

## Skill config file

`~/.nullclaw/skills/cct2/config.json` — overrides model defaults:
```json
{
  "primary_provider": "anthropic-custom:minimax",
  "primary_model": "MiniMax-M3",
  "backup_provider": "glm-direct",
  "backup_model": "GLM-5.1"
}
```

If `backup_model` returns 429 (overload), the script automatically retries with `glm-4-flash` before giving up.

## Market time

Both reports are named for a **session**, so every date and hour in them is US
Eastern — the ET trading day, derived in `America/New_York` and never from a
UTC date. The header carries the session date and the market-time stamp
(`📊 CCT2 收盤報告｜2026-08-12 16:10 EDT`), so a reader in any zone can tell
which session the report belongs to.

`--et-hour H` gates the run on the market-time hour and exits `ok` without
delivering when it does not match. This is how an ET-pinned job survives DST:
the scheduler fires on a fixed UTC expression, so each job is scheduled at
**both** UTC hours the year can put it at and the wrong one skips. Same
mechanism as doughcon.

## Pre-market prediction journal

A pre-market run writes `~/.nullclaw/skills/cct2/journal/<ET-date>.json` —
each ticker's direction, confidence, and the price the model was shown. The
end-of-day run for the same trading day reads it back, compares each call to
the close, and opens the report with a **🔁 盤前預測覆盤** section before the
day's own analysis.

A call counts as right when the close clears ±0.5% in the predicted direction;
`neutral` is right inside that band. Every actual percentage is printed beside
its verdict, so the band is visible rather than assumed. A prediction with no
direction, or with a price missing on either side, is marked ➖ and excluded
from the tally rather than counted as wrong.

A missing journal is normal — the pre-market run may have been skipped or
failed — and renders no review section at all, which is a different statement
from an empty one. The journal is written only when the run produced rows, so
a failed run never overwrites a good record.

## Which models answered

**Every** run — both modes, whether or not it produced rows — appends one JSON
line to `~/.nullclaw/skills/cct2/journal/models.jsonl`:

```json
{"business_date":"2026-08-27","mode":"eod","made_at":"16:10 EDT","requested":5,
 "primary":{"model":"MiniMax-M3","answered":true,"tickers":5},
 "backup":{"model":"GLM-5.1","answered":true,"tickers":5}}
```

A model can fail without the run failing: when one returns nothing the report
ships on the other, foots itself `單一模型回應` instead of `雙模型對照`, and
still exits `[skill-status:ok]`. That is the intended behaviour, but the only
live trace is a `WARN` on stderr and nothing keeps stderr — `cron_runs.output`
holds the marker lines and no more. This file is the record, so "how often did
the primary go quiet" is a `grep`, not an inference.

`answered` and `tickers` are separate on purpose: a model can reply and still
omit a ticker, so the report is a true two-model comparison only when both
counts reach `requested`.

```bash
# runs where a model went quiet
grep -c '"answered":false' ~/.nullclaw/skills/cct2/journal/models.jsonl
```

Appended, never rewritten: a retry is a second fact, not a correction of the
first, and the day's prediction file is left alone.

## Cron jobs

Scheduled in **UTC** (`--tz +00:00`), paired per report so exactly one of each
pair survives the `--et-hour` gate in either half of the year.

```
# pre-market — ET 08:30, one hour before the open
nullclaw cron add-skill "30 12 * * 1-5" cct2 --tz +00:00 --verify skill_contract --repair retry_once --timeout 300 --skill-args "--mode pre-market --et-hour 8" --deliver-to 7972814626
nullclaw cron add-skill "30 13 * * 1-5" cct2 --tz +00:00 --verify skill_contract --repair retry_once --timeout 300 --skill-args "--mode pre-market --et-hour 8" --deliver-to 7972814626

# eod — ET 16:10, forty minutes after the close
nullclaw cron add-skill "10 20 * * 1-5" cct2 --tz +00:00 --verify skill_contract --repair retry_once --timeout 300 --skill-args "--mode eod --et-hour 16" --deliver-to 7972814626
nullclaw cron add-skill "10 21 * * 1-5" cct2 --tz +00:00 --verify skill_contract --repair retry_once --timeout 300 --skill-args "--mode eod --et-hour 16" --deliver-to 7972814626
```

Before 2026-08-13 the two jobs ran at Taipei 13:35 and 21:10, which is **ET
01:35 and 09:10** — the "pre-market" report was produced in the middle of the
American night and the "收盤報告" twenty minutes *before* the open. Yahoo's
daily bar for the session had not been created at either time, so both reports
read the *previous* session's close: two reports over the same numbers, with no
trading in between. That is why the close could only ever restate a forecast,
and it is what the journal plus this schedule fix.
