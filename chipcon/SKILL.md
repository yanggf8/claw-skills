---
name: chipcon
description: Observe SMH semiconductor momentum via trend and relative-strength signals; signal-only observation report (no trade or exit advice).
always: true
---

# chipcon

Observe semiconductor **market momentum** (SMH vs 20/50DMA and relative
strength vs QQQ/SOXX). This is **observation-only**: no entry/exit advice, no
position sizing, no portfolio edits. Status labels describe trend health, not
an instruction to sell or reduce.

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

| Status | Meaning (observation only) |
|---|---|
| `OK` | Trend intact. |
| `YELLOW` | Momentum weakening (observe). |
| `ORANGE` | Trend deterioration (observe). |
| `RED` | Trend broken vs key averages / RS (observe). |
| `PROFIT_PROTECT` | Still extended above 20DMA after a down day (observe). |
| `INSUFFICIENT_HISTORY` | Need more history before trend can be trusted. |

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
