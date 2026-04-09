---
name: cct
description: Fetch CCT 4-moment trading intelligence and deliver to Telegram
always: true
---

# cct

Fetch CCT (Capital Cloudflare Trading) 4-moment market intelligence and deliver to Telegram.

## Script

```
~/.nullclaw/skills/cct/scripts/run.py
```

## Usage

```
python3 ~/.nullclaw/skills/cct/scripts/run.py --mode pre-market
python3 ~/.nullclaw/skills/cct/scripts/run.py --mode intraday
python3 ~/.nullclaw/skills/cct/scripts/run.py --mode eod
python3 ~/.nullclaw/skills/cct/scripts/run.py --mode weekly
python3 ~/.nullclaw/skills/cct/scripts/run.py --mode pre-market --deliver-to 7972814626
```

## Options

- `--mode MODE` — pre-market, intraday, eod, weekly (required)
- `--deliver-to CHAT_ID` — Send output to Telegram chat instead of stdout
- `--account NAME` — Telegram account name from config (default: main)

## Cron Schedule

```bash
# Pre-market: 8:35 AM EST = 13:35 UTC weekdays
nullclaw cron add-skill "35 13 * * 1-5" cct --deliver-to 7972814626 --skill-args "--mode pre-market"

# Intraday: 12:05 PM EST = 17:05 UTC weekdays
nullclaw cron add-skill "5 17 * * 1-5" cct --deliver-to 7972814626 --skill-args "--mode intraday"

# EOD: 4:10 PM EST = 21:10 UTC weekdays
nullclaw cron add-skill "10 21 * * 1-5" cct --deliver-to 7972814626 --skill-args "--mode eod"

# Weekly: Sunday 10:05 AM EST = 15:05 UTC
nullclaw cron add-skill "5 15 * * 0" cct --deliver-to 7972814626 --skill-args "--mode weekly"
```

## Output Format

**Pre-market:**
```
📊 CCT 盤前報告｜2026-04-08

市場情緒：看漲 🟢（信心 75%）
分析標的：12 支

🎯 高信心訊號（≥70%）
  • NVDA 看漲 92% — Data center demand accelerating
  • AAPL 看漲 85% — Services revenue beat expectations
  • MSFT 看漲 78% — Azure cloud growth outperforming
```

**EOD:**
```
📊 CCT 收盤報告｜2026-04-08

今日總結：看漲 🟢（信心 71%）
分析標的：12 支
看漲 8 支｜看跌 3 支｜中性 1 支

🎯 高信心訊號
  • NVDA 看漲 89% — Continued momentum from earnings
明日展望：看漲（信心 68%）
```

## Notes

- API: `https://tft-trading-system.yanggf.workers.dev`
- Auth: `X-API-Key` header — read from config `cct.api_key`, fallback `yanggf`
- On API error or empty cache: prints/sends honest status message, exits 0
- Source of truth is D1; DO is read-through cache only
