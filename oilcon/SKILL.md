---
name: oilcon
description: Fetch oil futures levels and deliver or record a daily regime snapshot
always: true
---

# oilcon

Fetch WTI, Brent, and heating-oil closes, then deliver a compact oil regime snapshot or record it to a history log.

## Script

```
~/.nullclaw/skills/oilcon/bin/oilcon
```

## Usage

```
~/.nullclaw/skills/oilcon/bin/oilcon
~/.nullclaw/skills/oilcon/bin/oilcon --mode record
~/.nullclaw/skills/oilcon/bin/oilcon --deliver-to CHAT_ID
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

### JETS rule-review flag (deliver mode only)

When WTI is in a *sustained uptrend* — defined as **≥10% above its recent low, that low ≥30 days ago, and still rising** (current close > mean of the last 5 closes) — deliver mode appends one advisory line:

```
⚠ JETS: oil in sustained uptrend (WTI +20.0% off low, low 49d ago, rising) — review entry-exit-rules.md JETS Reduce Rule
```

This is a **rule-review prompt, not a recommendation.** It surfaces that a JETS reduce condition may be met and points to the human's written rule; it never says buy/sell and has no portfolio awareness. The thresholds are constants in `run.py` (`JETS_OFF_LOW_PCT`, `JETS_MIN_DAYS_SINCE_LOW`, `JETS_RISING_WINDOW`). The line is omitted when the condition is not met, and record mode is unaffected.

## Record output

```
2026-04-15 17:00:01 CST  WTI 78.20  high 92.40@2026-01-08 (-15.4%)  low 74.10@2026-03-02 (+5.5%)  BZ +0.7% HO +0.8%
```

## Notes

- Data source: Yahoo Finance chart API
- Store: Turso/libsql via `TURSO_DATABASE_URL` and `TURSO_AUTH_TOKEN`
- Deliver mode emits `[skill-status:ok]` for fresh data and `[skill-status:degraded]` for warning/stale output, then `[trace:<NULLCLAW_JOB_ID>]` on separate stdout lines
- Record mode emits `[skill-status:ok]` only after the history log append succeeds
