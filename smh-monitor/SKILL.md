---
name: smh-monitor
description: Monitor the manually approved SMH tactical position, report price-trigger status, and remind the operator to review semiconductor/AI-capex event risks before acting.
always: false
---

# smh-monitor

Monitor the SMH high-beta semiconductor satellite position. This skill is
signal-only: it does not trade, does not edit portfolio state, and does not
approve broker captures.

## Script

```bash
~/.nullclaw/skills/smh-monitor/scripts/run.py
```

## Usage

Dry run from the source tree:

```bash
python3 ~/a/claw-skills/smh-monitor/scripts/run.py --config ~/a/claw-skills/smh-monitor/config.example.json --current 607.81
```

Runtime run:

```bash
python3 ~/.nullclaw/skills/smh-monitor/scripts/run.py
```

## Configuration

Runtime config lives at:

```text
~/.nullclaw/skills/smh-monitor/config.json
```

Required fields:

- `ticker`: normally `SMH`
- `position_usd`: intended or filled dollar position size
- `chat_id`: Telegram target for NullClaw cron metadata
- `enabled`: set `true` only after SMH is actually filled
- `entry_price`: filled SMH entry price; required only when `enabled` is true

## Trigger Rules

Price triggers are measured from the filled entry price:

| Trigger | Status |
|---|---|
| -8% | `REVIEW` |
| -12% | `REDUCE_REVIEW` |
| -15% | `EXIT_BIAS` |
| +15% | `RAISE_STOP` |
| +25% | `TAKE_PROFIT_1` |
| +35% | `TAKE_PROFIT_2` |

Event risks are manual-review reminders only:

- NVIDIA / Broadcom / AMD / Micron guidance
- TSMC monthly revenue
- Microsoft / Amazon / Google / Meta capex guidance
- Export-control escalation
- SpaceX IPO / index-flow liquidity drain

## Cron

Let NullClaw handle Telegram delivery; the script prints stdout only.

Example daily Taiwan-time runs:

```bash
nullclaw cron add-skill "30 5 * * 2-6" smh-monitor --deliver-to 7972814626 --timeout 120 --tz +08:00 --verify skill_contract --repair retry_once
nullclaw cron add-skill "15 21 * * 1-5" smh-monitor --deliver-to 7972814626 --timeout 120 --tz +08:00 --verify skill_contract --repair retry_once
```

The skill emits `[skill-status:ok|failed]` and `[trace:<job_id>]` for
`skill_contract` verification.
