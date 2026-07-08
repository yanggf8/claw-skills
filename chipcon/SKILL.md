---
name: chipcon
description: Monitor semiconductor momentum for the SMH tactical position using trend and relative-strength signals, then deliver a signal-only exit-review report.
always: true
---

# chipcon

Monitor the SMH semiconductor satellite position using trend signals, not
entry-price stop levels. This skill is signal-only: it never trades and never
edits portfolio state.

Data input comes from Yahoo Finance. Each daily run fetches ~1 year of daily
closes per configured ticker via `lib/oil_fetch.py` `fetch_history(range=1y)`
for the trend calculation. No local store, no registry, and no broker state is
read or changed.

## Script

```
~/.nullclaw/skills/chipcon/scripts/run.py
```

## Usage

```
python3 ~/.nullclaw/skills/chipcon/scripts/run.py
python3 ~/.nullclaw/skills/chipcon/scripts/run.py --mode record
python3 ~/.nullclaw/skills/chipcon/scripts/run.py --deliver-to 7972814626
```

## Trend Signals

The report watches:

- SMH close vs 20DMA and 50DMA
- 20DMA direction over a 5-trading-day lookback
- SMH 5-day relative strength vs QQQ
- SMH 5-day confirmation vs SOXX
- consecutive SMH down days
- overextended SMH above 20DMA

Status ladder:

| Status | Meaning |
|---|---|
| `OK` | Trend intact. |
| `YELLOW` | Momentum weakening; do not add automatically. |
| `ORANGE` | Trend deterioration; reduce review. |
| `RED` | Exit-bias review. |
| `PROFIT_PROTECT` | Trend still high but overextended; protect gains review. |
| `INSUFFICIENT_HISTORY` | Need more local history before trend can be trusted. |

Manual event checks remain outside the algorithm:

- NVIDIA / Broadcom / AMD / Micron guidance
- TSMC monthly revenue
- Microsoft / Amazon / Google / Meta capex guidance
- Export-control escalation
- SpaceX IPO / index-flow liquidity drain

## Data Store

- Source: Yahoo Finance chart API via `lib/oil_fetch.py` `fetch_history(range=1y)`
- Each run fetches ~1 year of daily closes per configured ticker (SMH, QQQ, SOXX)
- No local store and no price registry — history is fetched fresh each run

## Cron

Daily after the US close, Tuesday-Saturday Taiwan time:

```
nullclaw cron add-skill "30 5 * * 2-6" chipcon --deliver-to 7972814626 --timeout 180 --tz +08:00 --verify skill_contract --repair retry_once
```

The skill emits `[skill-status:ok|degraded|failed]` and `[trace:<job_id>]`
for `skill_contract` verification.

## Delivery

The Telegram report is sent as **plain text** (`parse_mode=None`). The body
is not Markdown — the status string `INSUFFICIENT_HISTORY` contains an
underscore, and upstream WARN text can carry unbalanced backticks/brackets.
Under Telegram legacy Markdown these break entity parsing (`can't parse
entities`, HTTP 400) and the delivery fails. Plain text is both correct and
immune to any content.

## Degraded vs failed

- `degraded` (`skill-status:degraded`): a Yahoo fetch WARN was present but the
  report still built and delivered — e.g. a secondary ticker (QQQ or SOXX)
  failed or returned no rows while SMH succeeded. The run still delivers; it
  self-reports `degraded` to flag the fetch warning.
- `failed` (`skill-status:failed`): a hard error — SMH fetch failed or empty,
  or delivery failure.
