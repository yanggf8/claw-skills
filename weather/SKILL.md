---
name: weather
description: Fetch weather forecast for Taiwan (CWA) and Hong Kong (HKO)
always: true
---

# weather

Fetch weather forecast for one or more locations (CWA for Taiwan, HKO for Hong Kong).

## Script

```
~/.nullclaw/skills/weather/bin/weather
```

## Usage

```
~/.nullclaw/skills/weather/bin/weather --location 新北市 --location 臺北市
~/.nullclaw/skills/weather/bin/weather --location 香港
~/.nullclaw/skills/weather/bin/weather --location 香港 --location 臺北市
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

- Taiwan locations: CWA API (`CWA_API_KEY` from `~/.nullclaw/.env`), with Open-Meteo as automatic fallback when CWA is down (timeout/5xx/empty records). Fallback lines are suffixed with `（備援）`.
- Hong Kong locations: HKO API, no key required
- Location routing is automatic based on name matching
- CWA timeout is 8s so fallback engages quickly; Open-Meteo needs no API key

## Fallback observability

When the Open-Meteo fallback fires, three signals are emitted:

1. **Stderr** — a single self-contained `[skill-event]` sentence naming primary, fallback, plain-language reason, scope, and elapsed ms. Example:
   ```
   [skill-event] Weather skill fell back from CWA to Open-Meteo because CWA request failed with TimeoutError: The read operation timed out. Fallback covered 1 Taiwan location and took 927ms.
   ```
   Audience is an agent reading the trace; phrasing is natural language, not key=val.
2. **Trace status** — when `NULLCLAW_JOB_ID` is set, `[skill-status:degraded]` is emitted instead of `[skill-status:ok]` so cron run history surfaces the fallback without parsing the event line.
3. **User message** — only the `（備援）` suffix on affected location lines; no error noise pushed to Telegram.
- Telegram bot token loaded from `~/.nullclaw/config.json`
- On API error: prints/sends `[WARN: weather unavailable - <reason>]`, exits 0
- Selects the forecast period closest to current CST/HKT time
- Cron verification: use scheduler-owned `skill_contract` with `retry_once`
- After delivery confirmation, cron runs emit `[skill-status:ok|failed]` and `[trace:<NULLCLAW_JOB_ID>]` on separate stdout lines
