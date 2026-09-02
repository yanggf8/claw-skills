# cct: the producer runs on a trigger with no bound on its start time

> 中文一句：cct 四份報告的「產生端」挂在 GitHub Actions `schedule:` 上，而那個觸發不保證準時
> （實測延遲到 +10.1 小時）；讀取端是固定 UTC cron，緩衝只有 3 小時，所以一定會有日子落空。
> 本文件把證據、三個修法與驗收條件寫下來，**請 owner 選一個**——尚未動任何生產端。

- **Filed:** 2026-09-02
- **Decision (2026-09-02): option B — trigger from the box that reads the reports.**
  Four nullclaw shell jobs (`tools/trigger-cct-job.py`) at the same cron times;
  the Actions `schedule:` block was removed from `trading-system.yml`
  (`yanggf8/cct@c663fa8`), keeping `workflow_dispatch`. Watchdog scheduled as
  `job-e05f83c8` (`5 0 * * *` UTC). Full rationale: comment on this issue.
- **Repo changed:** both — `yanggf8/cct` (producer trigger, workflow) and
  `yanggf8/claw-skills` (trigger tool, watchdog, docs).
- **Severity:** product degraded, no data lost — four consecutive trading mornings of stale/empty pushes (2026-08-27 → 09-01).

## 1. Symptom

```
09-01 15:35Z  pre-market  [WARN: CCT pre-market carries no analysis] stale: payload date=2026-08-31 today=2026-09-01 age=1d
09-01 19:05Z  intraday    [cct] first fetch unusable, retrying once after 60s...
09-01 19:05Z  intraday    [WARN: CCT intraday carries no analysis] the worker has no intraday content for 2026-09-01
```

Run history for the two cct read jobs (`~/.nullclaw/cron.db`): `ok` through 08-26, then
`contract_degraded` on 08-27, 08-28, 08-31 and 09-01 for **both** pre-market and
intraday, while `eod` returned to `ok` on 08-31 and 09-01.

## 2. Measured evidence

The producer chain is: GitHub Actions `schedule:` →
`.github/workflows/trading-system.yml` → `POST /api/v1/jobs/trigger` → worker writes a
D1 snapshot. `GET /api/v1/jobs/runs` returns when each job actually started; joining
that against the four cron times (12:30 / 16:00 / 20:05 UTC weekdays, 14:00 Sunday)
gives the drift of the *trigger itself*:

| UTC date | pre-market | intraday | end-of-day | weekly |
|---|---|---|---|---|
| 08-24 … 08-26 (baseline) | +0.9 … +1.1 h | +0.6 … +0.9 h | +0.6 h (08-26 **+3.3**) | +0.3 h (08-23) |
| 08-27 | **+10.0 h** | +8.8 h (partial → stamped 08-28) | +8.3 h (failed at `data_fetch`) | — |
| 08-28 | **+10.1 h** | +8.5 h (partial) | +6.7 h (failed) | +3.9 h (08-30) |
| 08-31 | **+6.7 h** | +5.4 h | +3.7 h | — |
| 09-01 | **+4.3 h** | +3.3 h | +2.5 h | — |

Rules out, with sources:

- **Not the queue.** Each late run has `created_at == run_started_at`, `run_attempt: 1`,
  and job durations of 0.4–10 min (`GET /repos/yanggf8/cct/actions/runs`), so they were
  not waiting on each other or on a runner.
- **Not billing.** `yanggf8/cct` is public (`private: false`); Actions minutes are not
  the constraint, and nothing was cancelled.
- **Not the worker.** `GET /api/v1/jobs/schedule-check` reports `allPresent: true` for
  2026-09-01 with `latestStatus: success` on all three daily types — the day's reports
  were written, at 16:52Z / 19:21Z / 22:32Z.
- **Not this box, not Telegram, not cct2.** Same host, same network, same account:
  `cct2` (which generates its own analysis) recorded `pre-market` and `eod` answered by
  both models on 08-28, 08-31 and 09-01 (`cct2/journal/models.jsonl`).

GitHub documents `schedule:` as best-effort and delayed during high load; community
reports put the delay at 8–14 hours with a staff reply confirming multi-hour backlogs.
This is the trigger behaving as designed, in a design that cannot carry a deadline.

## 3. The contract that is broken

A cron read absorbs a *fixed* amount of upstream lateness:

| read | nominal generator | actual read (live cron) | buffer | drift on 09-01 | result |
|---|---|---|---|---|---|
| pre-market | 12:30Z | 15:35Z | 3 h 05 m | 4 h 20 m | **missed** |
| intraday | 16:00Z | 19:05Z | 3 h 05 m | 3 h 19 m | **missed** |
| eod | 20:05Z | 23:45Z | 3 h 40 m | 2 h 27 m | ok |

One incident, three outcomes, because only the buffers differ. `HISTORY.md` (2026-08-27)
already said it: *a fixed time is a buffer, not a bound*. This is the invoice.

The skill is not at fault and needs no change: it retried once (60 s, sized for the
10-minute race of 08-26), degraded with a reason, and delivered yesterday's snapshot
labelled stale. That is the contract working.

## 4. Options

### A. Put the trigger where the clock is — Cloudflare cron triggers

**This is closer to config-only than it looks.** `src/modules/scheduler.ts:177-205`
already dispatches on the *scheduled time* (`utcHour`/`utcMinute`/`utcDay`) and knows all
five modes — it does not read `event.cron`. `wrangler.toml:100-105` carries `crons = []`
with the comment "Cloudflare cron had 3-schedule limit and $0.20/month DO cost".

- **A1 (what the comment in `wrangler.toml` assumes, checked 2026-09-02):** Cloudflare's
  current limits page puts **cron triggers at 5 per *account* on Workers Free and 250 on
  Paid** — not three per worker. The four modes fit on the free plan as four verbatim
  expressions, so no folding and no plan change. The "3-schedule limit" note that motivated
  the 2025 migration to Actions is out of date and is the reason this option was written off.
- **The limit that actually binds A: wall-clock per Cron Trigger invocation is 15 minutes**
  (HTTP invocations have no wall-clock limit — that is what the Actions path uses today via
  `--max-time 600`). Observed worker-side job times from the run rows are 6 s (`eod`) to
  3 m 40 s (`pre-market` on 08-31), so roughly 4x headroom; the 10-minute outlier in the
  Actions list is the *workflow* (checkout + `npm ci` + curl), not the job.
- **CPU is not a reason to stay.** Per invocation the cron budget equals the HTTP budget on
  Free (10 ms) and *exceeds* it on Paid (15 min CPU for triggers spaced >= 1 h apart, versus
  the 30 s HTTP default). This pipeline is I/O-bound on provider and LLM calls, which do not
  count as CPU.
- **What it still costs:** the Durable Object write cost named in the same comment — worth
  re-reading, because the scheduled path may not touch a DO at all now; and the loss of the
  Teams notification plus the `health-check` job that the workflow wraps around the run,
  which is the observability the migration bought.
- **Which plan the account is on was not verified** — `GET /accounts/{id}/subscriptions`
  with the stored wrangler OAuth token returned `10000 Authentication error`, so it needs a
  fresh `wrangler login`. It changes the CPU margin, not the shape of the choice.

### B. Trigger from the box that already keeps the reads honest

A `nullclaw cron` shell job doing `POST /api/v1/jobs/trigger` at 12:30 / 16:00 / 20:05 UTC
weekdays and 14:00 Sunday. Evidence for this scheduler: 44 jobs, 40 enabled, all four cct
reads fired **at their scheduled minute** on 09-01. No CF cost, keeps the existing
observability, and the producer lands on the same clock as the consumer.

- **Risk:** the box reboots (the daemon's current process started 08-28 21:33), and a down
  machine at 12:30Z is a missing pre-market. Leaving Actions on as a redundant trigger is
  survivable but not free, and the mechanics are now checked rather than assumed:
  `generateRunId` mints `${date}_${type}_${uuid4}` and `job_run_results.run_id` is the
  primary key, so two triggers for one day write **two run rows** (the history stops being
  one row per day, which is what `check-cct-generator.py` reads). `job_date_results` is
  upserted on `(scheduled_date, report_type)` with `executed_at = NULL` and
  `errors_json = NULL`, so the later trigger **rewinds the day's status to `running`** and
  repoints `latest_run_id`. The report routes only read `job_date_results` when no snapshot
  exists (`report-routes.ts:1226-1265`), so an existing report keeps being served — what
  changes is the empty-state message, from "Run POST /api/v1/jobs/intraday to generate" to
  "job is running … retry in a moment", and the duplicated provider/LLM spend.

### C. Move the reads later to absorb the drift

Rejected. The pre-market read at 15:35Z already lands **after** the 09:30 ET open in either
half of the year (11:35 EDT / 10:35 EST). Buying another two hours of buffer would ship a
report labelled 盤前 in the American afternoon, which is the one thing the report exists to
not be.

## 5. Interim mitigation already in place

`tools/check-cct-generator.py` (claw-skills `763a12e`) measures this drift and exits
non-zero on a missing run, a non-`success` status, drift over `--grace` (2 h default), or a
run that landed after the read that needed it; the read times come from `cron.db`, so the
check cannot drift out of sync with the schedule. Verified on the live capture:
09-01 → three findings naming both missed reads; 08-25 → green.

It is scheduled as `job-e05f83c8` at `5 0 * * *` UTC — after the last read of the ET day,
before the first of the next — the same shape as cct2's durable check (`30 0 * * *`,
`tools/check-cct2-models.py`).

## 6. Acceptance criteria (for A or B)

1. 10 consecutive trading days with the watchdog green: every run starts within 2 h of its
   nominal cron.
2. No `contract_degraded` on pre-market or intraday attributable to trigger drift for those
   days. A stale payload should then mean a real upstream failure again.
3. The pre-market row is written **before 13:30Z** (09:30 ET open) at least 9 days in 10 —
   the actual service objective, and the only change that lets the read move back toward
   13:35Z and be a pre-bell briefing again.
4. `eod` unaffected (it already lands inside its buffer), and the D1 row for a day is written
   exactly once (no double-run duplication).

## 7. Out of scope here (tracked separately)

- Six `news` jobs disagree on `tz_offset_s` (two at +08:00, four at 0 with hour fields that
  read like Taipei time), so the account-topic digests ship at 16:33/16:36 Taipei.
- `maybeAlertSkillStreak` in nullclaw has no log line on any of its early returns, so "did
  the streak fire" cannot be answered from disk. Post-mortem context: `HISTORY.md`
  (2026-08-28, 2026-09-02).

## 8. References

- `HISTORY.md` — `## cct: the producer moved eight hours and the reader had no idea (2026-09-02)`
- `cct/SKILL.md` — *Why the reads carry a 3-hour buffer*
- `~/a/cct/wrangler.toml:100-105`, `~/a/cct/src/modules/scheduler.ts:177-205`,
  `~/a/cct/.github/workflows/trading-system.yml:9-17`
- Evidence endpoints: `GET /api/v1/jobs/runs`, `GET /api/v1/jobs/schedule-check`;
  Actions history via `GET /repos/yanggf8/cct/actions/runs`
- Trigger limits: `developers.cloudflare.com/workers/platform/limits/` (read 2026-09-02) —
  "Number of Cron Triggers per account: 5 Free / 250 Paid", "Cron Trigger duration 15 min"
- Duplicate-run mechanics: `src/modules/d1-job-storage.ts:940-987` (`generateRunId`,
  `startJobRun`), `src/routes/report-routes.ts:1226-1265` (when `job_date_results` is read)
- Skill behaviour: `crates/cct/src/main.rs` (in-run retry), `crates/cct/src/freshness.rs`,
  `crates/cct/src/content.rs`
