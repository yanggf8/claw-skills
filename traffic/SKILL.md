---
name: traffic
description: Fetch TomTom route travel time between waypoints
always: true
---

# traffic

Fetch TomTom route travel time between two or three waypoints.

## Script

```
~/.nullclaw/skills/traffic/scripts/run.py
```

## Usage

```
python3 ~/.nullclaw/skills/traffic/scripts/run.py --from 淡水安泰登峰 --to 小巨蛋
python3 ~/.nullclaw/skills/traffic/scripts/run.py --from 士林 --via 昌吉街重北路口 --to 淡水安泰登峰
```

## Options

- `--from LOCATION` — Origin (required)
- `--to LOCATION` — Destination (required)
- `--via LOCATION` — Optional waypoint (max 1; TomTom free tier: 2–3 waypoints total)
- `--deliver-to CHAT_ID` — Send output directly to Telegram chat instead of printing to stdout

Location names are resolved via `~/.nullclaw/locations.json`. Raw `lat,lon` strings also accepted.

## Output

```
🚗 淡水安泰登峰→小巨蛋：32分鐘
```

## Notes

- API key loaded from `~/.nullclaw/.env` (`TOMTOM_API_KEY`)
- Telegram bot token loaded from `~/.nullclaw/config.json`
- On API error: prints/sends `[WARN: traffic unavailable - <reason>]`, exits 0
