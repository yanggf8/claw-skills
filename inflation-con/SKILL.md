---
name: inflation-con
description: Judge whether inflation is genuinely persistent (not one hot print) from FRED core-PCE / core-CPI / breakeven data, apply the written status ladder, and deliver a signal-only inflation-confirmation evidence packet.
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
