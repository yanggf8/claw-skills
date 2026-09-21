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

The four expressions below are **the live jobs**, read out of `~/.nullclaw/cron.db`
on 2026-09-02 (`select expression, skill_args from cron_jobs where
skill_name='cct'`). They are fixed UTC, so the ET time each one lands at **moves
with daylight saving** — but so does the generator's cron, so the *buffer*
between the two is DST-stable even though the wall-clock reading is not.

```bash
# Pre-market: generator cron is 12:30 UTC (08:30 ET); read 3 h 05 m later.
nullclaw cron add-skill "35 15 * * 1-5" cct --deliver-to 7972814626 --skill-args "--mode pre-market"

# Intraday: generator cron 16:00 UTC; read 3 h 05 m later.
nullclaw cron add-skill "5 19 * * 1-5" cct --deliver-to 7972814626 --skill-args "--mode intraday"

# EOD: generator cron 20:05 UTC; read 3 h 40 m later, 300 s timeout for the in-run retry.
nullclaw cron add-skill "45 23 * * 1-5" cct --timeout 300 --deliver-to 7972814626 --skill-args "--mode eod"

# Weekly: generator cron 14:00 UTC Sunday; read 3 h 05 m later.
nullclaw cron add-skill "5 17 * * 0" cct --deliver-to 7972814626 --skill-args "--mode weekly"
```

### The producer is the worker itself (since 2026-09-14)

The schedule behind the four reports has moved twice. GitHub Actions
`schedule:` carried it until 2026-09-02 (best-effort, drifted +0.6…+1.1 h
normally and **+10.1 h** on 2026-08-28 — the drift table below); the box's
nullclaw cron carried it for the next two weeks, until a 29.4 h box outage on
2026-09-11 missed every Friday slot and the recovery drain wrote
Saturday-dated content ([HISTORY.md](HISTORY.md), 2026-09-14). The schedule
now lives where the reports live — `yanggf8/cct` `wrangler.toml
[triggers]`, four UTC crons (`30 12 * * 1-5` / `0 16 * * 1-5` / `5 20 * * 1-5`
/ `0 14 * * SUN`) dispatched by `scheduler.ts` on exact (hour, minute,
weekday) tuples. Do **not** add a second writer anywhere (Actions
`schedule:`, a box-side trigger job): one day would get two trigger rows
(`job_run_results.run_id` embeds a uuid4; `job_date_results` rewinds the day
to `running`).

Two helpers ship beside the reader (`tools/install-skill.sh cct` publishes
them together with it):

```bash
# Manual fallback / catch-up. POSTs the same /api/v1/jobs/trigger; refuses
# the daily modes on ET weekends and NYSE holidays — the 09-12 Saturday
# drain is what wrote the ghost report — and --dry-run prints the decision.
~/.nullclaw/skills/cct/bin/cct-trigger pre-market

# The watchdog: the recurring 5 0 * * * UTC shell job after the last read of
# the ET day (job-e05f83c8). Reads /api/v1/jobs/runs, measures each trigger
# against its nominal minute and the live read time from cron.db, and looks
# one trading day back (plus the Sunday weekly from a Mon/Tue vantage) —
# the 09-11 hole stayed invisible for three days because its only witness
# sat on the box that died. Exits non-zero on a missing run, a non-success
# status, drift over --grace (2 h default), or a run that landed after the
# read that needed it.
nullclaw cron add "5 0 * * *" "/home/yanggf/.nullclaw/skills/cct/bin/cct-check" --tz +00:00 --verify exit_only
```

```bash
~/.nullclaw/skills/cct/bin/cct-check                     # the current ET trading day
~/.nullclaw/skills/cct/bin/cct-check --date 2026-09-12   # replays the 09-11 hole
```

It sends the skill's own `User-Agent: nullclaw-cct/1.0`, because the WAF in
front of the worker 403s other user agents and 200s that one.

### Why the reads carry a 3-hour buffer

The buffer was sized while GitHub Actions produced the reports
(`yanggf8/cct`, `.github/workflows/trading-system.yml`, four `schedule:`
crons at 12:30 / 16:00 / 20:05 UTC weekdays and 14:00 Sunday, POSTing
`/api/v1/jobs/trigger`). Actions `schedule:` is documented best effort — it
delays during high load — and the delay was measured, not theorised:

| window | drift of each trigger from its cron time |
|---|---|
| baseline (through 2026-08-26) | **+0.6 … +1.1 h**, steady |
| 2026-08-27 / 08-28 | **+8.3 … +10.1 h** |
| 2026-08-31 | +3.7 … +6.7 h |
| 2026-09-01 | +2.5 … +4.3 h |

A read buffer of 3 h absorbs everything up to 3 h of drift and nothing beyond it:
on 2026-09-01 pre-market needed 4 h 20 m and intraday 3 h 19 m, so both
degraded, while `eod` (3 h 40 m of buffer, 2 h 27 m of drift) delivered. That
asymmetry is the whole design: the buffer is sized for the baseline plus a
margin, and a producer that drifts further than the margin is an *upstream*
incident, correctly reported as `degraded` with a reason rather than retried
into the void.

Two consequences worth stating plainly:

- The 15:35Z pre-market read lands **after** the 09:30 ET open in either half of
  the year (11:35 EDT / 10:35 EST). It is today's outlook, not a pre-bell edge.
  The 13:35Z reading this file used to document sat 5 minutes behind the
  generator's *nominal* time and inside its *actual* arrival distribution — the
  buffer was bought by moving the read, and that trade is the operator's.
- **A fixed read time is a buffer, not a bound.** Nothing here makes the
  generator arrive; `cct-check` exists so the drift itself is visible on the
  morning it happens instead of being inferred from four unexplained
  degradations. The recurring shell job at `5 0 * * *` UTC — after the last
  read of the ET day, before the next one — turns "the trigger is drifting
  again" into a single alert instead of three degradations with no shared
  explanation.


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
