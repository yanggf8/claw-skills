---
name: cct
description: Fetch CCT 4-moment trading intelligence and deliver to Telegram
always: true
---

# cct

Fetch CCT (Capital Cloudflare Trading) 4-moment market intelligence and deliver to Telegram.

## Script

```
~/.nullclaw/skills/cct/bin/cct
```

## Usage

```
~/.nullclaw/skills/cct/bin/cct --mode pre-market
~/.nullclaw/skills/cct/bin/cct --mode intraday
~/.nullclaw/skills/cct/bin/cct --mode eod
~/.nullclaw/skills/cct/bin/cct --mode weekly
~/.nullclaw/skills/cct/bin/cct --mode pre-market --deliver-to 7972814626
```

## Options

- `--mode MODE` — pre-market, intraday, eod, weekly (required)
- `--deliver-to CHAT_ID` — Send output to Telegram chat instead of stdout
- `--account NAME` — Telegram account name from config (default: main)

## Cron Schedule

The cron expressions are fixed UTC, so the ET time each one lands at **moves
with daylight saving**: the comments below are the EST (winter) reading, and
every one of them is an hour later in ET from March to November. Check against
the market clock before treating any of them as "shortly after the close".

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

## Delivery Contract

Standard nullclaw skill contract (`skill_contract` verification), same as the
claw-skills siblings:

- Delivery via `delivery.deliver_or_fail()` — on Telegram failure the body is
  preserved on **stdout**, the diagnostic goes to **stderr**, and the skill
  **exits 1**. Markers are not emitted in that case, so a delivery failure stays
  a hard exec error rather than a semantic verification failure.
- On success: `[skill-status:ok|degraded]` then `[trace:<job id>]` on stdout.
  Both are no-ops unless `NULLCLAW_JOB_ID` is set, so manual runs stay clean.
- `ok` = CCT returned a report with substantive content. `degraded` = anything
  else that still delivered: CCT unreachable, or a payload that is empty /
  placeholder / job-failed.
- **pre-market additionally requires the content to be current**: `ok` = content
  **and** not stale. The pre-market route falls back to the latest D1 snapshot
  when today's job never ran, so a payload can carry a full set of signals and
  still describe a market day weeks back. `pre_market_freshness()` treats a
  payload as stale when `is_stale` is set, when the date is absent or
  unparseable, or when the date is not today. Stale is `degraded`, not
  `failed` — a retry returns the same snapshot.

  **Which "today" depends on where the date came from.** The worker publishes
  `metadata.business_date` on the envelope — an **ET** business date, because ET
  is the market's own time — and that is compared against today in
  `America/New_York`. When the field is absent, the skill falls back to the
  payload's own `date`, which is what the route served before it learned the
  difference, and compares it against today in **UTC**. `comparison_today()`
  holds that rule. Binding the clock to the field, rather than switching
  globally, is what lets this skill and the worker deploy in either order: for
  the four to five hours after 00:00 UTC the two calendars name different days,
  so a global switch would call fresh reports stale from whichever side ran
  ahead.
  The delivered header then carries the *source* date plus a warning:
  `📊 CCT 盤前報告｜2026-06-08  ⚠️ 資料已過期（50 天前）`, or
  `⚠️ 資料已過期` with no day count when the age is not a positive number of
  days, or `日期不明` when the payload has no usable date.

  The distinction matters because the API answers HTTP 200 + `success: true`
  even when a job never ran or failed outright (`report-routes.ts` turns
  `jobStatus.status === 'failed'` into a success envelope carrying only a
  `message`). Keying status off "did a payload arrive" would report `ok` while
  the pipeline is broken, so each mode has a substantive-content predicate —
  `has_pre_market_data()` etc. — because every empty state has a different
  shape (pre-market/intraday zero counters + `message`; eod zeroes a nested
  counter with no `message`; weekly drops `report` entirely).

  **eod serves two different shapes.** The real report is a prediction
  *scorecard* — flat camelCase (`modelGrade`, `correctCalls`/`wrongCalls`,
  `signalBreakdown`, `topLosers`, `tomorrowOutlook`, top-level
  `symbols_analyzed`) and it carries **no `daily_summary` at all**. The
  `daily_summary` shape is only ever the placeholder `report-routes.ts`
  synthesises when it finds no snapshot for the requested date. Testing for
  `daily_summary.symbols_analyzed` alone therefore reported `degraded` on every
  genuine report; `has_eod_data()` and `format_eod()` now accept both, and
  `eod_session_date()` prefers `metadata.business_date` and only falls through
  the payload's own timestamps when the worker has not published it. The
  fallback is kept but is a guess chain, and one of its links is worse than a
  guess: `timestamp` is an ISO **UTC** instant, so truncating it to ten
  characters prints a UTC day for a session that closed the evening before.
  Fixture: `crates/cct/tests/eod_scorecard.json`, captured from the live API.

  Empty payloads are `degraded`, not `failed`: `failed` triggers repair/retry,
  but retrying cannot produce a report that was never generated — that fix
  belongs in the CCT job pipeline.

- `get()` also rejects an explicit inner `success: false`. The weekly route
  serves a DO-cache miss as outer `success: true` wrapping
  `{success: false, error: ...}`; without the check that object flows through
  as data and the skill delivers an empty report header.
- Diagnostics (`[WARN: CCT ...]`) go to stderr — stdout is body + markers only.

### Runtime dependencies

None beyond the binary. Delivery and the scheduler markers come from
`claw-core`, linked in at build time — this skill used to reach back into
`claw-skills/lib` for them at import time through a three-step path search,
which is why the skill moved into this repo before it was ported.

## Notes

- API: `https://tft-trading-system.yanggf.workers.dev`
- Auth: `X-API-Key` header — read from config `cct.api_key`, fallback `yanggf`.
  Config path: `$CLAW_CONFIG`, else `~/.nullclaw/config.json`
- On API error or empty cache: sends honest status message, `degraded`, exits 0
- Source of truth is D1; DO is read-through cache only
