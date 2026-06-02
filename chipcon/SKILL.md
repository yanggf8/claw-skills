---
name: chipcon
description: Monitor semiconductor momentum for the SMH tactical position using trend and relative-strength signals, then deliver a signal-only exit-review report.
always: true
---

# chipcon

Monitor the SMH semiconductor satellite position using trend signals, not
entry-price stop levels. This skill is signal-only: it never trades and never
edits portfolio state.

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

## Cron

Daily after the US close, Tuesday-Saturday Taiwan time:

```
nullclaw cron add-skill "30 5 * * 2-6" chipcon --deliver-to 7972814626 --timeout 180 --tz +08:00 --verify skill_contract --repair retry_once
```

The skill emits `[skill-status:ok|degraded|failed]` and `[trace:<job_id>]`
for `skill_contract` verification.
