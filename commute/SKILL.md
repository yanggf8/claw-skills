---
name: commute
description: Fetch route travel time via traffic sub-skill
always: true
---

# commute

Traffic-only skill: fetches route travel time via the traffic sub-skill.

Weather is handled by a separate standalone cron job to avoid repetition
when multiple commute legs fire in sequence.

## Script

```
~/.nullclaw/skills/commute/scripts/run.py
```

## Usage

```
python3 ~/.nullclaw/skills/commute/scripts/run.py \
  --from 淡水安泰登峰 --to 小巨蛋
```

## Options

- `--from LOCATION` — Origin (required)
- `--to LOCATION` — Destination (required)
- `--via LOCATION` — Optional waypoint
- `--location LOCATION` — (ignored, kept for backward compat)
- `--deliver-to CHAT_ID` — Send output directly to Telegram chat instead of printing to stdout

## Output

```
🚗 淡水安泰登峰→小巨蛋：32分鐘
```

## Notes

- Delegates to `traffic/scripts/run.py` via subprocess
- Per-subprocess timeout: 30 seconds
- Subprocess stderr appended to `~/.nullclaw/skill-errors.log`
- Telegram bot token loaded from `~/.nullclaw/config.json`
- Always exits 0 (best-effort delivery)
- If traffic fails: outputs `[traffic unavailable]`
- Cron verification: use scheduler-owned `skill_contract` with `retry_once`
- Emits `[skill-status:ok]` for real route output and `[skill-status:degraded]` for warning/fallback output, then `[trace:<NULLCLAW_JOB_ID>]` on a separate stdout line
