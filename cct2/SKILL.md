---
name: cct2
description: Dual-LLM market sentiment analysis for configured tickers — pre-market and EOD reports delivered to Telegram
always: true
---

# cct2

Fetches prices and headlines from Yahoo Finance, runs sentiment analysis in parallel with two LLMs (primary + backup), and delivers a report to Telegram. Flags tickers where the two models disagree.

When running under cron, the `NULLCLAW_JOB_ID` is appended to the Telegram message and prefixed on every log line — matching the news skill's traceability format.

## Script

```
~/.nullclaw/skills/cct2/scripts/run.py
```

## Usage

```
python3 ~/.nullclaw/skills/cct2/scripts/run.py --mode pre-market
python3 ~/.nullclaw/skills/cct2/scripts/run.py --mode eod
python3 ~/.nullclaw/skills/cct2/scripts/run.py --mode pre-market --deliver-to 7972814626
python3 ~/.nullclaw/skills/cct2/scripts/run.py --mode eod --deliver-to 7972814626 --account main
```

## Options

- `--mode MODE` — pre-market | eod (required)
- `--deliver-to CHAT_ID` — Send output to Telegram chat instead of stdout
- `--account ACCOUNT` — Telegram account name (default: main)

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
  "primary_model": "MiniMax-M2.7",
  "backup_provider": "glm-direct",
  "backup_model": "GLM-5.1"
}
```

If `backup_model` returns 429 (overload), the script automatically retries with `glm-4-flash` before giving up.

## Cron jobs

```
nullclaw cron add-skill "35 13 * * 1-5" cct2 --skill-args "--mode pre-market" --deliver-to 7972814626
nullclaw cron add-skill "10 21 * * 1-5" cct2 --skill-args "--mode eod" --deliver-to 7972814626
```
