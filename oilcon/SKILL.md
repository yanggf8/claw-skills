---
name: oilcon
description: Fetch oil futures levels and deliver or record a daily regime snapshot
always: true
---

# oilcon

Fetch WTI, Brent, and heating-oil closes, then deliver a compact oil regime snapshot or record it to a history log.

## Script

```
~/.nullclaw/skills/oilcon/scripts/run.py
```

## Usage

```
python3 ~/.nullclaw/skills/oilcon/scripts/run.py
python3 ~/.nullclaw/skills/oilcon/scripts/run.py --mode record
python3 ~/.nullclaw/skills/oilcon/scripts/run.py --deliver-to CHAT_ID
```

## Options

- `--mode deliver` — Print/send formatted output (default). Exits 0 on degraded upstream states.
- `--mode record` — Append one line to `~/.nullclaw/oilcon-history.log`. Exits non-0 on failure.
- `--deliver-to CHAT_ID` — Send output directly to Telegram chat instead of printing to stdout.
- `--account NAME` — Telegram account selector (default `main`).

## Deliver output

```
🛢️ OILCON 情報
WTI: $78.20 (+0.9%)
  高 $92.40 (2026-01-08, 97日前, -15.4%)
  低 $74.10 (2026-03-02, 44日前, +5.5% 離低點)
確認：Brent ✓ (+0.7%)   HO ✓ (+0.8%)
更新：2026-04-15 17:00
```

## Record output

```
2026-04-15 17:00:01 CST  WTI 78.20  high 92.40@2026-01-08 (-15.4%)  low 74.10@2026-03-02 (+5.5%)  BZ +0.7% HO +0.8%
```

## Notes

- Data source: Yahoo Finance chart API
- Store: Turso/libsql via `TURSO_DATABASE_URL` and `TURSO_AUTH_TOKEN`
- Deliver mode emits `[skill-status:ok]` for fresh data and `[skill-status:degraded]` for warning/stale output, then `[trace:<NULLCLAW_JOB_ID>]` on separate stdout lines
- Record mode emits `[skill-status:ok]` only after the history log append succeeds
