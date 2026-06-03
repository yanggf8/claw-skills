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
- `--et-hour H` — Only run when the current US-Eastern hour equals `H` (0–23); otherwise exit 0 as a clean no-op (still emits `[skill-status:ok]` + trace). Lets a UTC cron follow US daylight saving: schedule both `00:00` and `01:00` UTC with `--et-hour 20` and exactly one firing passes each day (00:00 in summer/EDT, 01:00 in winter/EST). Omit to run unconditionally.

## Deliver output

```
🍕 DOUGHCON 情報
目前等級：DOUGHCON 3
指數：7.42
更新：2026-03-24 11:23 CST（美東 03-23 23:23 EDT）
```

The `更新` line shows the API's own snapshot time (`timestamp` field), not when the
script ran. The US-East line auto-switches EDT/EST via `zoneinfo`, so you can see
which US-Eastern hour the data actually covers. Falls back to the run time only if
the API returns no usable timestamp.

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
- DST-aware scheduling: PizzINT tracks Pentagon-area pizza demand, so the useful
  window is US-Eastern evening (≈18:00–20:00 ET). `nullclaw cron` uses fixed UTC
  offsets and can't follow daylight saving, so each job is scheduled twice in UTC
  (`00:00` + `01:00`) with `--et-hour 20`; the gate keeps only the firing that
  lands on ET 20:00 (CST 08:00 in summer, CST 09:00 in winter)
