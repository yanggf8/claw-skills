---
name: doughcon
description: Fetch PizzINT DOUGHCON level and deliver or record
always: true
---

# doughcon

Fetch PizzINT DOUGHCON level — deliver a formatted message or record to history log.

## Script

```
~/.nullclaw/skills/doughcon/scripts/run.py
```

## Usage

```
python3 ~/.nullclaw/skills/doughcon/scripts/run.py              # deliver (default)
python3 ~/.nullclaw/skills/doughcon/scripts/run.py --mode record
```

## Options

- `--mode deliver` — Print/send formatted output (default). Exits 0 always (warns on failure).
- `--mode record` — Append one line to `~/.nullclaw/doughcon-history.log`. Exits non-0 on failure.
- `--deliver-to CHAT_ID` — Send output directly to Telegram chat instead of printing to stdout

## Deliver output

```
🍕 DOUGHCON 情報
目前等級：DOUGHCON 3
指數：7.42
更新：2026-03-24 08:00
```

## Record output (to log file)

```
2026-03-24 20:00:01 CST  DOUGHCON 3  index=7.42
```

## Notes

- API: `https://pizzint.watch/api/dashboard-data`
- No API key required
- Telegram bot token loaded from `~/.nullclaw/config.json`
- record mode exits non-0 on API failure (gap is detectable via cron `last_status`)
- Cron verification: use scheduler-owned `skill_contract` with `retry_once`
- Deliver mode emits `[skill-status:ok]` for real data and `[skill-status:degraded]` for warning/no-data output, then `[trace:<NULLCLAW_JOB_ID>]` on a separate stdout line
- Record mode emits `[skill-status:ok]` only after the history log append succeeds
