---
name: inflation-con
description: Judge whether inflation is genuinely persistent (not one hot print) from FRED core-PCE / core-CPI / breakeven data, apply the written status ladder, and deliver a signal-only inflation-confirmation evidence packet. Also carries the fed-braking watch (doc 191 mechanical half): a daily DFEDTARU change detector plus the monthly EFFR +100bp/6m rule arithmetic.
always: true
---

# inflation-con

Monitor whether inflation is **genuinely persistent** — a confirmed
inflation-up regime, not a single hot print — so a portfolio review (add an
inflation hedge? revisit the IEF duration gate, decision #14?) is triggered by
evidence, not a hunch. This skill is **signal-only**: it classifies evidence
and emits a regime label; it never trades, never prescribes an action, and
never edits portfolio state.

The canonical rule (indicator table + status ladder) lives in the
finance-engineering repo at `risk-dashboard.md` → "Inflation Confirmation
Criterion". This skill is the optional monitor that delivers that evidence
packet monthly so releases aren't missed.

## Data input

FRED, via the public `fredgraph.csv` endpoint — **no API key** (unlike FRED's
JSON web-service API, which requires a 32-char key). CSV also satisfies the
agent-first / NO-JSON rule: `crates/inflation-con/src/fetch.rs` is the transport
adapter; the rest of the skill sees only `(date, value)` rows.

Series:

| Key | FRED series | Role |
|---|---|---|
| core_pce | `PCEPILFE` | **Primary** — the Fed's benchmark |
| core_cpi | `CPILFESL` | Confirmation |
| headline_pce / headline_cpi | `PCEPI` / `CPIAUCSL` | Context only |
| breakeven_10y | `T10YIE` | Market-priced expectations (daily) |
| real_yield_10y / nominal_10y | `DFII10` / `DGS10` | Rate context (daily) |

No local store and no price registry — series are fetched fresh each run.

## Script

```
~/.nullclaw/skills/inflation-con/bin/inflation-con
```

## Usage

```
~/.nullclaw/skills/inflation-con/bin/inflation-con
~/.nullclaw/skills/inflation-con/bin/inflation-con --mode record
~/.nullclaw/skills/inflation-con/bin/inflation-con --mode braking
~/.nullclaw/skills/inflation-con/bin/inflation-con --deliver-to 7972814626
```

## Status ladder

Core PCE is the primary metric (the Fed's preferred gauge). Headline CPI/PCE
is context only — an energy spike is real pain but not necessarily persistent
monetary inflation. 3-mo and 6-mo figures are compound-annualized.

| Status | Condition |
|---|---|
| `OK` | Core PCE 3-mo annualized < 2.5% and 6-mo < 2.75%, or the trend is falling. |
| `WATCH` | One hot print / mixed: core PCE 3-mo >= 2.5% but 6-mo not confirming, or core CPI hot while core PCE is not. |
| `YELLOW` | Persistent above-target: core PCE 3-mo and 6-mo >= 3.0%, and core CPI also >= 3.0% (3-mo or 6-mo). |
| `RED` | Inflation-up confirmed: core PCE 3-mo and 6-mo both >= 3.5%, core CPI confirms, and context is not easing (10Y breakeven >= 2.5% or rising ~3 months, and policy stance not `easing`). |
| `INSUFFICIENT_DATA` | < 7 monthly core-PCE observations or the latest core PCE/CPI is missing. |

**FOMC policy stance is a manual config input**
(`restrictive | neutral | easing | unclear`) — never machine-parsed from Fed
text. It only tips the RED context clause. Update it by hand after each FOMC
meeting.

**Config file (runtime, not committed):**

- Path (absolute): `~/.nullclaw/skills/inflation-con/config.json`
- On a symlink deploy this is the same file as `inflation-con/config.json` in the
  repo (gitignored). Template: `config.example.json`.
- Missing file → loader defaults `policy_stance` to `unclear` (no silent
  fallback if the file exists but is corrupt JSON — that raises).
- Override: `--config /path/to/config.json`

## Fed-braking watch (`--mode braking`, doc 191 mechanical half)

The slot-5 research (finance-engineering doc 191) settled a composite watch
condition: **泡沫語境持續(人讀)× EFFR 6 個月 +100bp(機械)**. This skill
automates **only the mechanical half**; the bubble-context half stays
human-read and is never evaluated here.

Two series, two different roles — they are deliberately NOT interchangeable:

| Series | FRED | Role |
|---|---|---|
| `FEDFUNDS` | Monthly EFFR average | **Rule series** — month-index arithmetic: latest minus the observation six months earlier, shown against the reference line `+100bp`. A daily-series approximation is a different measure and is never presented as this rule. |
| `DFEDTARU` | Daily target-range upper bound | **Timeliness series** — daily change detection surfaces a Fed move weeks before the monthly average can. (DFEDTAR is discontinued 2008; DFEDTARL is the lower bound.) |

Change detection keeps a persisted cursor
(`~/.nullclaw/skills/inflation-con/braking-cursor.json`, gitignored, written
through the install symlink):

- **first run** = baseline: records the latest (date, value), delivers nothing
  (history is not news);
- **unchanged** = no-op line on stdout, exit 0;
- **changed** (including a round-trip: hike then cut back) = delivery, and the
  cursor advances **only after a successful delivery** — a failed send must
  re-report on the next run;
- a present-but-corrupt cursor is a hard error, never a silent re-baseline.

In `--mode braking`, DFEDTARU is the hard-fail primary (like core_pce in the
monthly mode); FEDFUNDS failing only renders `n/a` for the rule.

The monthly report (default mode) also appends a `──── Fed braking` section
with the rule arithmetic. A braking-series fetch failure omits the section and
leaves the inflation status untouched — the two are separate concerns (Codex
review 2026-10-02) and a missing add-on must not read as a degraded
inflation run.

**No ladder in the braking output.** Unlike the inflation status table, the
braking half renders arithmetic plus a stated reference line and nothing else —
no OK/WATCH/YELLOW/RED, no 成立/觸發, no advice. A percentile-over-window or
threshold reading is a judgment the window flips; the tool states the numbers,
the human reads them. Same discipline as `cds-con`.

## Frequency

Monthly, not daily. Inflation is not a daily signal. Best cadence: run the day
after each CPI release (~mid-month) and the day after each PCE release
(~month-end), plus a manual policy-stance note after FOMC meetings.

**Live cron** (added 2026-07-08, job `skill-d8960d53`):

```
nullclaw cron add-skill "0 6 3-5 * *" inflation-con --deliver-to 7972814626 --timeout 180 --tz +08:00 --verify skill_contract --repair retry_once
```

Runs 06:00 on days 3–5 of each month, UTC+8. The early-month window catches
the prior month's PCE release; the run no-ops usefully if data hasn't updated —
it just reports the latest available. Next fire after wiring: 2026-08-03.

**Daily braking cron** (doc 191 mechanical half, added 2026-10-02):

```
nullclaw cron add-skill "40 6 * * *" inflation-con --skill-args "--mode braking" --deliver-to 7972814626 --timeout 180 --tz +08:00 --verify skill_contract --repair retry_once
```

Daily at 06:40 UTC+8, after the cds-con store write at 06:00 and the cds-con
skill at 06:30. Most days are a no-op (target unchanged); a DFEDTARU move
delivers the same day.

The skill emits `[skill-status:ok|degraded|failed]` and `[trace:<job_id>]`
for `skill_contract` verification.

## Boundary (hard)

**The monitor may classify evidence; it may NOT prescribe portfolio action.**

Allowed: `status = RED`, `regime = inflation-up confirmed`, "core PCE 3m/6m
confirms persistent pressure", "manual review: IEF gate / inflation-hedge gap".

Forbidden: "buy gold", "un-gate IEF", "allocate 10% to commodities", any
shares/dollars/target, any automatic plan-status change, any logic asserting a
plan condition is satisfied. The human decides, records a `finance-cli
decision add`, and verifies broker state.

## Delivery

Plain text (`parse_mode=None`) — status names carry underscores
(`INSUFFICIENT_DATA`) and FRED WARN text is arbitrary; both break Telegram
legacy Markdown entity parsing. Nothing in the body is intentional Markdown.

## Degraded vs failed

- `degraded`: a secondary/context series (e.g. `DGS10`) failed or returned no
  rows, but core PCE succeeded and the report built and delivered.
- `failed`: core PCE (`PCEPILFE`) fetch failed or empty, or delivery failed.
