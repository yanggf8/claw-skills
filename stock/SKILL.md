---
name: stock
description: Fetch TWSE/HSI market indices and individual stock quotes
always: true
---

# stock

Fetch market indices (TWSE, HSI) and individual stock quotes.

## Script

```
~/.nullclaw/skills/stock/scripts/run.py
```

## Usage

```
python3 ~/.nullclaw/skills/stock/scripts/run.py
python3 ~/.nullclaw/skills/stock/scripts/run.py --market tw
python3 ~/.nullclaw/skills/stock/scripts/run.py --market hk
python3 ~/.nullclaw/skills/stock/scripts/run.py --symbol 2330
```

## Options

- `--market tw|hk|all` — Market to query (default: all)
- `--symbol SYMBOL` — Individual TWSE stock symbol (e.g. 2330 for TSMC)
- `--deliver-to CHAT_ID` — Send output directly to Telegram chat instead of printing to stdout
- `--account NAME` — Telegram account name from config (default: main)

## Output

```
📈 發行量加權股價指數：32518.16 -594.43 (-1.80%)
   高 33009.68 / 低 32326.37，20260330 13:33:00
📈 恒生指數：24750.79 +368.32 (+1.51%)
```

## Notes

- TWSE index and individual stocks via `mis.twse.com.tw` API
- HSI via Yahoo Finance API with HKEX fallback
- Telegram bot token loaded from `~/.nullclaw/config.json`
- On API error: prints/sends `[WARN: ... unavailable - <reason>]`, exits 0
- Market hours: TWSE 09:00-13:30 CST, HKEX 09:30-16:00 HKT
