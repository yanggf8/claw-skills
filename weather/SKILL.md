---
name: weather
description: Fetch weather forecast for Taiwan (CWA) and Hong Kong (HKO)
always: true
---

# weather

Fetch weather forecast for one or more locations (CWA for Taiwan, HKO for Hong Kong).

## Script

```
~/.nullclaw/skills/weather/scripts/run.py
```

## Usage

```
python3 ~/.nullclaw/skills/weather/scripts/run.py --location 新北市 --location 臺北市
python3 ~/.nullclaw/skills/weather/scripts/run.py --location 香港
python3 ~/.nullclaw/skills/weather/scripts/run.py --location 香港 --location 臺北市
```

## Options

- `--location LOCATION` — Location name (repeatable). Taiwan: `新北市`, `臺北市`, etc. Hong Kong: `香港`, `九龍`, `新界`, `港島`, `hk`
- `--deliver-to CHAT_ID` — Send output directly to Telegram chat instead of printing to stdout
- `--account NAME` — Telegram account name from config (default: main)

## Output

```
🌤 香港：大致多雲，有幾陣驟雨。低溫23°C / 高溫28°C，降雨概率中高
🌤 臺北市：晴時多雲，低溫20°C / 高溫28°C，降雨機率30%
```

## Notes

- Taiwan locations: CWA API, key from `~/.nullclaw/.env` (`CWA_API_KEY`)
- Hong Kong locations: HKO API, no key required
- Location routing is automatic based on name matching
- Telegram bot token loaded from `~/.nullclaw/config.json`
- On API error: prints/sends `[WARN: weather unavailable - <reason>]`, exits 0
- Selects the forecast period closest to current CST/HKT time
