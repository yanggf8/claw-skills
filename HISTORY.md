# HISTORY.md — retirements, migrations and post-mortems

Narrative **evicted from `CLAUDE.md`**: retired skills, the Python→Rust migration, and "how we found
out" write-ups. `CLAUDE.md` is loaded into every session in this repo, so it keeps only present-tense
rules — current config, live footguns, decision rules. This file is not loaded automatically.

Design specs live in `docs/specs/` and remain the authority for how a thing is *supposed* to work;
this file is only the record of what changed and why.

---

## cct: the box died with the watchdog in its lap, and the producer moved home again (2026-09-14)

Three Telegram degradations on a Saturday morning, about the previous day:

```
[cron] skill 'cct' degraded: failure=contract_degraded repair=none trace=…:4633
[WARN: CCT pre-market carries no analysis] stale: payload date=2026-09-10 today=2026-09-12 age=2d
```

**What happened.** The box crashed around 2026-09-11 01:00 UTC — wtmp ends in
`crash` after the morning's login activity — and came back through 57
one-minute reboot loops, stable at 09-12 06:23 UTC. The 29.4 h window
covered all three of Friday's generation slots (12:30 / 16:00 / 20:05 UTC),
all three Friday reads, *and* the Sat 00:05Z watchdog run that was the only
scheduled check able to see Friday's hole. On recovery nullclaw drained the
missed slots: the readers ran first — that is where the three degradations
came from — and the triggers then POSTed at 06:28–06:32Z, generating content
stamped **2026-09-12, a Saturday**, which the pre-market route then served
as its latest snapshot. The W37 weekly's daily ledger lists 09-12 and skips
09-11. The worker's own status endpoint had marked 09-11 `missed` from the
start; nothing read it.

**Three defects, not one.**
1. *Producer and watchdog shared one reliability domain.* The 09-02 migration
   put generation on the box that reads the reports (an explicit trade —
   "producer and consumer share one clock"), and the watchdog moved with it.
   One box death took out the producer *and* the only witness.
2. *The watchdog only ever asked about its own day.* Once the calendar rolled
   past 09-11, every later run said green
   (`no scheduled reports for 2026-09-12 (Sat)`). A hole behind a missed
   check stayed a hole forever.
3. *The catch-up drain had no trading-day gate.* The trigger POSTed whatever
   day it woke up on, the worker generated a Saturday report, and the
   reader's freshness rules — built for exactly this shape — correctly called
   it stale. The pipeline converted a dead day into noise plus a ghost
   report.

**The strategy change (owner's ruling): stop post-mortem handling; when the
cause is knowable, act immediately.** The producer moved to the worker
itself: `yanggf8/cct` `wrangler.toml [triggers]` now carries the four UTC
crons that `scheduler.ts` had kept matching since the CF era — the fallback
CLAUDE.md had pre-approved for "this box ever misses its minutes", enabled
the day that case actually happened (the 08-31 decision to tolerate GH drift
predated the box era and did not cover it). Two deployment facts worth
keeping: Cloudflare's cron parser wants `SUN`, not `0` (error 10100,
"invalid cron string"), and a failed trigger change **half-applies** rather
than rolling back — verify the deploy output's `schedule:` lines, not its
exit code.

The box kept the reads and gained a rebuilt watchdog. `crates/cct` ships
three binaries: `cct` (reader, contract unchanged), `cct-check` (the
watchdog), `cct-trigger` (the manual fallback). The Python originals
(`tools/check-cct-generator.py`, `tools/trigger-cct-job.py`, and the
`cct_calendar.py` helper) were deleted at the port: they were twelve days
old, and the standing instruction is that the whole stack is Rust — the
09-02 pair had been written in Python inside that instruction's shadow. The
port upgraded two things structurally rather than mechanically: the HTTP
calls went through `claw_core::http::agent`, so `tools/lint-http.sh` now
actually reaches the watchdog; and ET dates come from jiff's bundled tzdb,
so the checker's `UTC − 4h` fallback — off by an hour in EST, a variant of
the ET rule this repo already holds — is gone because no such fallback
exists to port.

**The watchdog looks backward now.** The backward check covers the previous
trading day's dailies, plus the Sunday weekly from a Monday/Tuesday vantage
(weekly's only other witness was the Monday 00:05Z run — the same
single-witness shape that hid 09-11). It reports the two hard verdicts only:
never ran, ended non-success. Rows are attributed by nearest nominal
trigger, not by the `scheduled_date` stamp — the worker stamps by fire time
and a late trigger crosses the boundary (adversarial-review finding;
`evaluate` had already learned this). An unresolved miss alerts on
consecutive runs until the calendar carries it out of range — deliberate;
the repeat is the recovery net. The reader grew the weekend gate
(`freshness::is_weekend`): a drained slot landing on an ET Saturday now
answers `市場休市` with `ok` markers, no fetch, no Telegram. The gate needed a
frozen clock for tests, so `CCT_TEST_NOW` exists — honored only when
`NULLCLAW_JOB_ID` is absent or the tests' `test-trace:1`, because dotenv
fills absent keys and a stray paste into `~/.nullclaw/.env` must not freeze
production's clock.

**The replay is the oracle.** Against the live runs history captured on
09-14: `cct-check --date 2026-09-12` flags the hole, `--date 2026-09-10`
stays green. One differential against the retired Python is recorded rather
than hidden: strict-stamp found three findings for 09-11, nearest-nominal
finds two — the Sat 06:32Z catch-up eod sits 10.5 h from Friday's nominal
and 13.5 h from Saturday's, so it attributes to Friday and the verdict
becomes "the slot got a run, 10.5 h late" where the Python's stamp reading
said "never ran". That semantics change is deliberate and the ghost content
it looked past is exactly what the trigger gate now prevents.

**Single-writer discipline.** The four box shell jobs were deleted, not
paused — a paused job is one cron.db restore away from a second trigger row
and a rewound day. Re-adding them is two commands in `cct/SKILL.md` if the
worker ever loses the schedule. The adversarial review also ordered the
cutover: deploy → verify the four `schedule:` lines → then delete the box
jobs, leaving no zero-writer window and no overlap before the 12:30Z slot
that would prove the new producer.

---

## oilcon: the defect rode in with a rebuild, and the probe caught a second one (2026-09-11)

Four identical alerts, four trading nights:

```
thread 'main' panicked at ~/.cargo/registry/.../libsql-0.6.0/src/hrana/mod.rs:307:55:
called `Option::unwrap()` on a `None` value
```

`exec_error` at 22:00 on 09-07, 09-08, 09-09 and 09-10, each run dead about two
seconds in, `repair=none` so the alert was the only noise. The last ok run was
09-04 22:00. The first failure came hours after the day's `oilcon` redeploy.

**The bug was never in this repo's diff.** gwebcdb's `f2c29ab` (08-25) taught
every price-store reader to fetch `adj_close` at column index 4 while leaving
five SELECTs naming four columns — `SELECT ticker, date, close, source`. libsql
diverges by transport on that mistake:

- **local** (`Builder::new_local`, every test): an out-of-range index reads as
  NULL, `FromValue for Option<f64>` turns it into `Ok(None)` — adj_close
  silently `None`, suite green. (The `Err(InvalidColumnIndex)` path exists only
  in `BatchedRow`, which serves `execute_batch`, not `conn.query`.)
- **hrana** (the remote Turso path the cron actually speaks): `Row::column_value`
  does `self.inner.get(idx).cloned().unwrap()` — panic, exit 101.

The suite stayed green for a specific reason worth naming:
`adj_close_round_trips` verified writes with its own raw `SELECT adj_close`
and only ever asserted `close` through the readers — the readers' adj_close
field was never asserted by anything. The red-then-green test that shipped with
the fix (`readers_surface_the_stored_adjustment_instead_of_a_silent_none`)
round-trips a stored adjustment through all five readers.

**Why 09-07 and not 08-26.** The 09-07 rebuild existed for this repo's own
one-line fix `5a954da` (adj_close: None at the StoredPrice initializer), but a
path dependency is a snapshot of the *dependency repo's* working tree at build
time: the rebuild pulled in every gwebcdb change since the previous binary,
defective readers included. A claw-skills commit is not the only thing that
changes a deployed skill — any rebuild ships the current gwebcdb tree with it.
The old binary ran clean through 09-04 because it predated f2c29ab entirely.

Fixed in gwebcdb `0557b56`: name `adj_close` fifth in every price SELECT (the
column list is the real guard), propagate get failures instead of swallowing
them, pin the round trip through every reader. The live registry was never at
risk of the missing-column error — the 09-07 runs died *after* `ensure_schema`
had already migrated it.

**Act two: the install probe refused the publish, and it was right twice.**
`tools/install-skill.sh` smoke-probes the staged binary with an unknown flag
and demands exit 2. From this session's shell it got exit 0 and the skill body
instead. Cause: `main()` connected to the price registry *before* `run()`'s
strict `parse_args` could refuse, and the no-registry path
(`dispatch_warning`) re-parses argv with a lenient `_ => {}` loop. No
credentials in the environment → connect fails → lenient parse → unknown flag
ignored → the body runs and exits 0. The 07-31 probe catch ("parser silently
ignored unknown flags") had only ever been fixed on the credentialed path —
the 09-07 install passed the probe because that shell held Turso tokens, not
because the contract held. The probe's verdict was environment-dependent all
along, which is itself the defect: the exit-2 contract must not depend on the
operator's shell. Fixed in `6884336`: strict argv validation before the
connect, and `tests/cli.rs` pins it offline by running the binary with the
credential env stripped — the probe can no longer pass for the wrong reason.

**The rules.** First: when a scheduled skill starts failing right after a
rebuild, diff the dependency repo's log against the old binary's mtime —
`git -C ~/b/gwebcdb log` explains failures no claw-skills diff can. Second: a
test gate over local connections cannot see hrana-only behavior; a reader
indexing past its SELECT is green offline and fatal on Turso, so round-trip
assertions must go through the readers, not around them. Third: a probe that
can pass for an environmental reason is not yet a probe — make the contract
hold with the environment stripped.

Verification 2026-09-11 10:15: deployed binary over real Turso + Yahoo, full
report, `rc=0` — first complete run since 09-04. `price` rebuilt from the same
fix (its `latest/at/history` carried the same mine). Declined for scope: Grok's
suggestion to read columns by name instead of index (the structural cure for
this drift class — worth its own task), and gwebcdb's pre-existing clippy debt.

## cct: the producer moved eight hours and the reader had no idea (2026-09-02)

Two alerts, four mornings apart from the same cause:

```
9/1 15:35Z  pre-market  [WARN: CCT pre-market carries no analysis] stale: payload date=2026-08-31 today=2026-09-01 age=1d
9/1 19:05Z  intraday    [cct] first fetch unusable, retrying once after 60s...
                         [WARN: CCT intraday carries no analysis] the worker has no intraday content for 2026-09-01
```

**The skill was right and nothing in it needed changing.** From 2026-08-27
through 09-01 cct degraded on every pre-market and intraday read — four
consecutive non-ok scheduled runs each — while `eod` went green on 08-31 and
09-01. Over the same days cct2, same box, same network, same Telegram route but
a generator it runs itself, recorded both modes answered by both models. The
fault is in neither the skill, the scheduler, nor the host.

**The producer started late.** Joining the worker's own `job_run_results`
(`GET /api/v1/jobs/runs`) against the four `schedule:` crons in
`~/a/cct/.github/workflows/trading-system.yml` (12:30 / 16:00 / 20:05 UTC
weekdays, 14:00 Sunday) gives each trigger's drift, in hours:

| UTC date | pre-market | intraday | end-of-day |
|---|---|---|---|
| 08-24 … 08-26 | +0.9 … +1.1 | +0.6 … +0.9 | +0.6 (08-26: **+3.3**) |
| 08-27 | **+10.0** | +8.8 | +8.3 (failed) |
| 08-28 | **+10.1** | +8.5 (partial) | +6.7 (failed) |
| 08-31 | **+6.7** | +5.4 | +3.7 |
| 09-01 | **+4.3** | +3.3 | +2.5 |

GitHub created each late run at its own start time (`created_at ==
run_started_at`, `run_attempt: 1`, no queueing; jobs lasted 0.4–10 min, so they
were not waiting on one another), and the repo is public, so Actions minutes are
not involved. This is `schedule:` behaving as documented — best effort, delayed
during high load, community reports of 8–14 hour delays, one GitHub staff reply
confirming a multi-hour backlog — decaying back toward its ~1 h baseline over
the week.

**Why the reads then fail.** The pre-market read sits 3 h 05 m after the
generator's cron, so it absorbs drift up to about 3 h; 09-01 needed 4 h 20 m.
The intraday read absorbs 3 h and 09-01 needed 3 h 19 m. `eod` reads 3 h 40 m
after its cron and that drift had already decayed under the margin — which is
exactly why `eod` looked healthy while its two siblings degraded: one incident,
three different buffers. The 60 s in-run retry is sized for the 10-minute race
it was written for (08-26: `eod` written 10 m 12 s after the read) and cannot
cross a gap measured in hours, nor was it meant to.

**The rule, third time.** `2026-08-27` ended with "a fixed time is a buffer, not
a bound". This is that sentence's full invoice: **a time-bounded consumer cannot
be paired with a best-effort producer.** The pre-market briefing has to be in
the reader's hands before 13:30Z (09:30 ET open); delivered at 16:52Z it is a
mid-session report wearing a 盤前 header, and the freshness contract — which is
what caught this — is the only reason four days of it did not read as silence.

**What changed here: `tools/check-cct-generator.py`.** Every number above came
from a hand-join across three sources, and the skill can only report what it saw
at read time — it cannot know that the row it wanted is being written right now.
The watchdog reports the drift itself and exits non-zero when a trigger is
missing, its status is not `success`, it lands later than `--grace` (2 h
default, against a ~1 h baseline), or it lands after the read that needed it.
The read times come out of `cron.db`, not a copy in the script: the cct schedule
has already drifted out of `SKILL.md` once, and a second place to keep current
is a second place to be wrong. Verified against a live capture — 2026-09-01
returns three findings naming both missed reads, 2026-08-25 returns green.

Two things it had to learn. The WAF in front of the worker answers **403** to
urllib's default `Python-urllib/3.x` and 200 to the skill's own
`nullclaw-cct/1.0` — same URL, same key, same minute — so a watchdog has to
carry a UA the path accepts. And when a trigger lands after midnight UTC the row
is filed under the *next* day's `scheduled_date` (08-28's 00:49Z intraday row is
08-27's 16:00Z trigger), so drift is measured against the nearest nominal
trigger and the output says which one it picked; a `-15.2 h` drift would read
like a clock bug instead of an eight-hour-late cron.

**Not fixed, by decision rather than oversight.** The trigger still has no bound
on its start time. Three ways to close it, all upstream or operator-side, none
of them a skill change:

1. Move generation off `schedule:` onto a Cloudflare cron trigger. `src/index.ts`
   already implements `scheduled()` → `handleScheduledEvent`, and `wrangler.toml`
   carries `crons = []` with a "3-schedule limit" comment that is still true on
   the free tier — so it fits as one folded trigger with hour-matching, or as a
   separate ticker worker.
2. Trigger from this box, whose cron demonstrably fires on the minute: all four
   cct reads ran at their scheduled minute on 09-01 (44 jobs, 40 enabled).
3. Move the reads later to absorb the drift — cheap, and it spends the one thing
   the report exists to provide. Not recommended.

**Follow-up (same day, evening): option B is the pick and it is in.** The box's
nullclaw cron now fires the four `/api/v1/jobs/trigger` POSTs at the same times
the Actions `schedule:` used — `tools/trigger-cct-job.py`, four shell jobs
(`job-6e97b576` / `job-c46b1ed4` / `job-d44da309` / `job-6236fec4`), verified
`exit_only` against the same pattern as the cct2 durable-check job, with the
trigger source header set to `nullclaw-cron`. `yanggf8/cct@c663fa8` removed the
`schedule:` block from `trading-system.yml` and kept `workflow_dispatch`, so a
day cannot get two trigger rows; the removal rationale lives in the workflow's
own header comment, and re-integration requires turning the box triggers off
first. The drift watchdog is scheduled as `job-e05f83c8`
(`5 0 * * *` UTC). Separately, the six `news` jobs disagreed on `tz_offset_s`:
the four `--account-topics` jobs stored Taipei hours as UTC and shipped at
16:33/16:36 Taipei; `nullclaw cron update --tz +08:00` makes it 08:33/08:36
(weekend 13:03), matching the two already-correct jobs.

**Open question worth a look.** `scheduler.alert_streak` is configured (3 →
`8768462400` via `nunu`) and the running binary postdates `2ae675fe`, so 08-31 —
the third consecutive non-ok scheduled pre-market run — should have escalated to
the operator chat independently of the job-first alerts. If it did not, the
streak path has a gap the scratch-daemon acceptance would not have caught: that
run exercised one synthetic missing-skill job, not four real sessions of a skill
that degrades *and* still delivers.

## news: by-topic theming graduated from shadow to render (2026-09-01)

The AI-section theming experiment (layer 6, `crates/news/src/theme.rs`) shipped
on 2026-07-23 with its own exit clause: run in `shadow` — classify, trace,
deliver byte-flat — and revisit the `ai_theme` trace once a few days of data
existed. The decision rule was written down then, before the data existed:
`其他` share median ≲ 25–30%; on days with ≥8 AI blocks, ≥1 theme with ≥2
stories at least ~half the time; shadow never perturbing delivery. Met →
`render`; not met → `off` and conclude.

Shadow ran 2026-07-23 .. 2026-08-31 and produced 44 `ai_theme` events:
28 classified, 10 `invalid_labels`, 4 `bad_result`, 2 `too_few_blocks`.

**Verdict: go.** `其他` share median **22.3%** (range 6–54%; the high tail is
real — six days sat at 38%+ — but the gate is on the median). Every classified
day carried 8–17 blocks, and **28 of 28** had at least one theme with ≥2
stories — the gate asked for half. The shape of the data also matches the
design's bet: 政策監管 is the workhorse (2–7 stories most days), while
研究突破 and 產品發表 arrive in bursts, exactly the kind of grouping a flat
digest cannot show. `NEWS_AI_THEME=render` was set on 2026-09-01 (host
`~/.nullclaw/.env`), effective from the next cron run; the binary is unchanged
and no redeploy was needed.

**The caveat that survived into render:** the classifier failed on 14 of the
44 events (10 `invalid_labels`, 4 `bad_result` ≈ 32%). Render fails open —
those days deliver the flat digest, unthemed but complete — so roughly a third
of days should be expected without `▸` headings. If `invalid_labels` clusters
after the flip, the classifier prompt or `parse_theme_response` is the place
to look, not the layout guards: the layout already proved itself on every
successful day.

The deferred optimization stays deferred: folding the classifier into the
existing half-select LLM calls (Codex's "Approach B", needing an
`AI_SUBSTAGE_CACHE_VARIANT` bump) is only worth doing if render sticks.

## news: the model's "nothing relevant today" answer was treated as a protocol violation (2026-09-01)

Six alerting runs between 2026-08-07 and 2026-08-31 — every one on 頂端客戶群
or 新品牌發表 — had the same shape, and the model had done nothing wrong.

The custom-topic prompt is alone among the four in offering an explicit escape
hatch: reply exactly 「- 今日無相關新聞」 when nothing qualifies. A model that
takes it emits zero news bullets, so `marker_stats` read `total == 0`, and
`run_custom_topic` returned that as a protocol violation. The caller
(`digest.rs`) then fell back to a raw listing of every candidate and alerted
the operator — publishing the very items the model had just rejected. Nothing
repairs it, either: the digest still shipped `ok`, so no scheduler retry ever
fired, and the same correct answer kept falling on the next thin day.

Counting honestly, because the first read was an undercount: the trace holds
79 `custom_topic_fell_back` events since it begins (2026-05-11) — 24 timeouts,
12 shape failures, 4 language failures, 38 marker failures. Only the marker
failures include the sentinel shape: 18 of them carry `marked=0/0` (頂端客戶群
×9, 財富傳承管理 ×3, 新品牌發表 ×2, 非營利組織/節稅/港股 ×1 each). The six
since 2026-08-07 were verified single-candidate (`items_numbered=1`), where
the sentinel is the only sensible reply; the earlier twelve — including 港股
on 2026-07-25 with eight candidates — share the shape in the trace but their
reply text was never recorded, so they cannot be classified retroactively.
The fix does not need the classification: a sentinel reply is honoured, and
anything else keeps falling back.

Two defects stacked. The first was the gate; the second was invisibility —
the custom-topic path returned its `Err` without ever calling
`log_validation_failed`, so the fallback event carried only the topic and the
error string. Diagnosing meant inferring the reply from the offered item
count instead of reading it.

**Fix, in `crates/news`:**

- `is_no_news_answer()` (`validate.rs`) recognises the sentinel and
  `run_custom_topic` honours it *before* the marker gate. The gate is
  deliberately narrow: every content line must be gone — a reply that answers
  and then argues has not answered the question and still falls back — and
  one line must reduce to exactly the sentinel body after peeling quotes,
  bullets and full stops, iteratively, because the prompt quotes the dash
  along with the body and one pass only removes the outer layer.
- A rejected reply is now logged verbatim (`llm_validation_failed` carries
  `stdout_sample`), so the next diagnosis reads the reply instead of
  inferring it.
- `NO_NEWS` moved beside the gates that filter it: `news_bullet_lines`,
  `content_lines` and `is_no_news_answer` all key off one definition, so the
  gate cannot drift from the filters — the failure mode where one counted the
  sentinel as content and the other dropped it is closed by construction.

**Follow-up (2026-09-02): the verbatim log paid for itself on its first day, and
the alert was still mute.** The 09-01 16:37 alert for 富人 was *not* the sentinel
class. `shape_validation`, with the reply now in the trace: `讓我分析這些候選新聞：`
followed by thirteen `- #N` lines that are editorial reasoning ("#1 和 #6 是同一
事件（同一篇「成功人士、富人有6個共同點」文章的不同來源）"), not the
`- #N 新聞標題` the prompt asks for. Every one of those bullets carries a legal
marker, so the marker gate passed and only the shape gate — the one pinned by
`reasoning_prose_is_invisible_to_the_bullet_list_but_visible_to_the_shape_gate` —
caught it. Right verdict, undiagnosable alert.

Counting the class turns one sentence into four diseases: 81
`custom_topics_fell_back` events since 2026-05-06 are 39 `marker_validation`,
24 `timeout`, 13 `shape_validation`, 4 `language_validation`, and the operator
has read all of them as "(LLM failed)" behind a single 30-day cluster counter.
The alert detail is now `富人=shape_validation; 節稅=timeout after 60s` (`98b5f08`).

Closed the same session, because the 09-01 alert proved it was a wrong shape,
not a non-answer: a `shape_validation` rejection (every bullet carrying a legal
marker) is retried once with a **rewritten** prompt — "你上次輸出的是分析與
評論，而不是選取結果；只輸出 - #N 標題" — under the same wall-clock budget
guard as the timeout retry (`crates/news/src/agent.rs::try_reformat_retry`;
traces `llm_agent_reformat_retry`, `custom_topic_reformat_retry_ok`). A second
bad answer still falls through to the same raw-listing fallback with the
original reply logged, so the gate's verdict is unchanged; the retry only
rescues the case the model can actually produce. `marker_validation
marked=N<M` (partial selections) and `language_validation` remain deterministic
same-prompt failures and are still not retried. The fallback's cost — it
publishes every candidate the model was asked to exclude — is unchanged.

## cct2: the timeout, and the primary model that was answering only sometimes (2026-08-27)

A `[cron] skill 'cct2' degraded: failure=timeout repair=retried_failed` alert on
2026-08-25 turned out to have two separate causes stacked on one job, and
finding the second one required not trusting the first fix.

**The timeout was a race the scheduler always wins.** `LLM_TIMEOUT_S = 120`
(`crates/cct2/src/llm.rs`) is the budget for *one* LLM call; the four cct2 cron
jobs carried `timeout_secs = 120` for the *whole run*. Equal budgets mean the
scheduler kills the skill at the exact moment the inner call is still entitled
to be waiting, so any slow upstream day is fatal. Nothing had changed in the
job settings — every cron-seed backup shows the same 120 — and runs from 08-13
to 08-24 all finished ok in 57–83 s. The 08-25 run took 241 s, which is
120 + 120 plus change: the original and the `retry_once` retry, both killed.

Two details worth keeping. The retry actually *completed its work* — the
journal for that day was written at `made_at 08:32 EDT` — and died before
delivery, so option A did its job and no half-finished report reached Telegram.
And resizing the budget was not possible without churn: `nullclaw cron update`
had no `--timeout`, so changing it meant remove + `add-skill`, which mints a new
job id and orphans that job's `cron_runs` history. That gap was closed on the
nullclaw side (`2d01f18f`, plus `84671712` / `a50560a2` bounding the value
everywhere it is parsed or loaded — `sqlite3_bind_int` takes a `c_int`, so an
over-range value used to trap mid-write rather than validate). The four jobs
now hold 300 s.

**The second cause was hiding behind an honest degradation.** Manual runs
showed `WARN primary: no text block (stop_reason=max_tokens, blocks=["thinking"])`
— MiniMax-M2.7 spending its entire `MAX_TOKENS = 4096` output budget on its
thinking block and returning no text at all. The report still shipped, footed
`單一模型回應` instead of `雙模型對照` (`render.rs:224`), rows non-empty,
`[skill-status:ok]`, no retry, no alert. The WARN is stderr-only and
`cron_runs.output` keeps just the ~74 bytes of marker lines, so none of this is
visible after the fact.

Measuring the frequency needed two tricks, because the obvious source is empty:

- **Live sampling.** Three `--mode eod` runs on 2026-08-27 gave 1 WARN in 3.
  So the first read — "reports have been backup-only since 08-26" — was
  overstated; it is intermittent, and an intermittent quality loss is exactly
  the kind that survives a spot check.
- **The journal, read sideways.** A consensus confidence can only end in `.xx5`
  if `merge.rs:94` averaged two models, so scanning `cct2/journal/*.json` for
  three-decimal values proves dual-voice on a past day at zero API cost. Five of
  the last ten trading days prove it, 2026-08-26 among them. Two-decimal values
  prove nothing either way — the heuristic has one direction only.

**Fix: primary switched to MiniMax-M3.** Raising `MAX_TOKENS` was the tempting
move and the wrong one — it had already gone 2048 → 4096 for this same symptom
during the port (the reasoning is still in the comment above the constant), and
thinking length has no ceiling to buy headroom against. The nullclaw agents had
moved to M3 on 2026-06-30 against the same thinking-stall symptom, so the
precedent was same model family, same failure, already resolved. Downside was
bounded: a model name the endpoint rejects degrades to backup-only, which was
the status quo.

Three runs after the switch: no WARN, `雙模型對照` all three times, and a real
divergence (NVDA, primary 看漲 85% vs backup 中性 65%) proving the primary was
genuinely answering rather than both paths falling to the backup. Runs also
dropped from 35–49 s to 9–11 s, which retires the original timeout race on its
own — the runaway thinking pass *was* the slow part.

`DEFAULT_PRIMARY_MODEL` moved with the config. It is only read when
`config.json` is missing, but leaving it on M2.7 would have quietly reinstated
the defect on any host without one (a fresh install, the nanoclaw container).

**The measurement problem is fixed too, which matters more than the model.**
Every run now appends a line to `journal/models.jsonl` naming each model and
whether it answered, so the next silent half-outage is a `grep` rather than the
`.xx5` inference above — that trick reads a past day at zero cost, but it can
only ever prove dual-voice, never its absence. The record went in the journal
directory and not in the day's prediction file: the close would have had to
read-modify-write that file to add its own line, and a crash in that window
trades a real prediction for "no review available". `answered` and per-model
`tickers` are separate fields because a model can reply and still omit a
ticker, which `answered` alone would report as a clean run.

**The record is now checked daily, not just written.** A durable shell job
`30 0 * * *` UTC (`job-fcd2b92d`, `tools/check-cct2-models.py`) exits non-zero
when the latest business day is missing either mode or a model went quiet, so
the silent half-outage this whole feature grew out of now surfaces as a cron
alert instead of a `grep` someone has to remember to run.

---

## cct: the eod read that raced the worker write (2026-08-27)

A `[cron] skill 'cct' degraded: failure=contract_degraded` alert carried
`[WARN: CCT eod carries no analysis] the worker has no eod content for
2026-08-26`. The WARN comes from `crates/cct/src/main.rs:119` and fires only
when `report.has_content == Some(false)`. That value is the worker's own
answer from the envelope's `metadata.has_content`, not a reader-side
heuristic — the skill did not misjudge.

The cct eod job `skill-a9cb5ac0` was scheduled at `10 23 * * 1-5` with
`tz_offset_s=0` (23:10 UTC). That run started `2026-08-26 23:10:14Z` and
finished in 4 s. The worker's row for business_date `2026-08-26` has
`_d1_created_at = 2026-08-26T23:20:26.502Z` — 10 m 12 s after the read.
Querying the worker now returns `metadata.has_content: true` for the same
day, so no data was lost.

Behaviour on this path is by design: `SkillStatus::Degraded` still delivers
(the stale-but-real report is footed `EOD analysis not yet available`),
`repair_policy=none` so there is no retry and no duplicate message, and
`cron_runs.output` keeps the body (cct has no `--deliver-to`, so stdout is
the record). Over the last 30 days cct had 12 degraded runs out of 66
spread across all four modes — not one cause — with three distinguishable
bodies in `cron_runs` (`EOD analysis not yet available`, `尚未產生或暫時
無法存取`, `No intraday data available`). The fix here addresses only the
first kind. The 2026-07-28 cluster (all three modes) was the day the
upstream pipeline was re-enabled after being auto-disabled for ~50 days.

**Fix: move the eod read from `10 23` to `45 23` UTC** via
`nullclaw cron update <id> --expression '45 23 * * 1-5'` — in-place, so the
job id and its `cron_runs` history survive. `23:45Z = 19:45 EDT (18:45 EST)`
is still the same UTC day and the same ET trading day, well clear of both
midnights, so `freshness.rs::comparison_today` is unaffected. The margin
rests on one observed worker completion time: the worker ignores `?date=` and
always serves the latest row, so no distribution of past completion times
could be obtained, and `2026-08-25` was `ok` at `23:10Z` — completion time
drifts and cct sits on the edge. A fixed time is a buffer, not a bound;
a durable fix would be the worker notifying or the skill retrying on
`has_content == false`, but that is a different kind of change.

---

## cct: the degradations nobody saw pile up, and the alert that was never configured (2026-08-28)

The eod post-mortem above closed with "a durable fix would be the worker
notifying or the skill retrying on `has_content == false`". Both halves of
that sentence happened, and the road between them surfaced a fault that had
been silent for as long as nullclaw has had an alert config.

**The durable fix arrived first.** `84aba5f` gave cct an in-run retry on
`has_content == Some(false)` — one 60 s wait, one refetch, `repair_policy=none`
so no scheduler retry can pair with it into a duplicate. Checking that against
the degraded history showed the retry was aimed at the rarer of two shapes: of
the 12 degraded runs on record, 3 were the not-written-yet shape but 5 were
`None` — transport/envelope failures landing on the "尚未產生或暫時無法存取"
placeholder, mostly a dropped connection or a 500 that recovers inside a
minute. `4cecb4b` extended the same retry to that case. A retry run typically
takes ~65 s against the 120 s job timeout (eod holds 300 s); only both fetches
burning the full 30 s read budget (~130 s) would be killed at 120 s, an
accepted edge. The 60 s sleep holds the sequential run queue — the price of
not using a scheduler retry, which would duplicate Telegram on degraded.

**The second half started with a wrong premise of mine.** The streak question
("cct keeps degrading — why does nobody see it pile up?") began with my claim
that skill degraded runs had no alert at all. Reading gateway.zig corrected
that: per-run degraded alerts exist, job-first. The corrected question — why
cct's repeated degradations never escalate to the operator — went to Codex,
and the root cause was not in any alerting logic: `config_parse.zig` never
parsed `scheduler.alert_channel` / `alert_to` / `alert_account`. The struct
fields existed, two read sites existed, tests set the fields by hand — but
nothing filled them from the file, so `state.alert_delivery` was permanently
null and every operator-fallback alert was silently dropped by
`deliverResult`. "The operator channel is configured" was true of the config
file and false of the binary. (`delivery.mode=always` jobs like cct still
alerted job-first, which is why cct's own chat kept receiving reports
throughout.)

**Fix: `2ae675fe` in nullclaw** — parse the three fields (non-empty gated,
the `claim_secret` precedent) plus a new `scheduler.alert_streak` (default 3,
0 disables); `detectRunStreak` counts consecutive non-ok scheduled runs over
`cron_runs`, edge-triggered (trigger exactly at `streak == N`, an ok run
re-arms); `maybeAlertSkillStreak` fires from all seven non-ok completion
paths, including the six pre-exec early exits that previously ended in
`complete(execErrorRunResult())` with no escalation. Operator chat: 8768462400
via account `nunu`; per-run alerts stay job-first. Reviews (Grok, then Codex
in four one-question delegations) earned their keep: the six early-exit
invokes, an id-anchored window replacing a wall-clock one, empty-string
gates, `.always` forced on the job-derived fallback — and, worst, that
`retry_once` writes two rows sharing a byte-identical trace id, so naive row
counting walks 2, 4, 6 and never hits 3; adjacent same-trace rows now collapse
into one logical run. My own first collapse implementation held a
`sqlite3_column_text` slice across steps — the row buffer is reused, so every
row compared equal to its neighbour and the streak was always 1. The tests
caught that one; the reviews did not.

**Acceptance found a second latent bug.** The procedure Codex drafted claimed
`NULLCLAW_HOME` isolates both config.json and cron.db. It isolates only
config.json: `cronDbPath()` composed `$HOME/.nullclaw/cron.db` while
`ensureCronDir()` created `defaultConfigDir()` — with `NULLCLAW_HOME` set
elsewhere the DB's parent was never created and every open failed with
`SqliteOpenFailed`, silently falling back to cron.json. The scratch daemon
read the real cron.db (43 jobs) for two minutes before the sha256 fingerprint
check proved it had written nothing. `f5641c9e` resolves both through the
same directory; production (`NULLCLAW_HOME` unset) is unaffected. The
acceptance itself — a HOME-isolated scratch daemon, a missing-skill job, and
per-run delivery pointed at a nonexistent account so only the operator-first
path could reach nunu — fired exactly one alert on the third consecutive
non-ok run, stayed quiet on the fourth, and left production cron.db
byte-identical.

Two habits this arc reinforced. `zig build test` alone silently skips every
sqlite test — `-Dengines=base,sqlite` is load-bearing, not optional. And a
delegated *procedure* is a draft until every command is verified against the
binary it will run: Codex's had one wrong premise and would have tripped its
own `test -f "$SCRATCH/cron.db"` step.

---

## cds-con: the spread renderer, retired (2026-08-13)

cds-con is the daily push for attribute 2 of the `~/b/finance-engineering`
research — "was corporate bond cost high or not". It had drifted into
measuring the wrong thing: not the Baa yield's own **level** (borrowing cost)
but the **direction** of the Baa−Aaa quality spread (how much more junk costs
than quality — credit stratification, a different question). A high-cost year
can be compressing and a low-cost year widening, and the direction measure
could not separate the anchors: five of six carried the same label, and 1966
(50th percentile) sat in the same class as 1999 (12th).

The owner's ruling of 2026-08-12 retired the charter and made `finance-cli`'s
`cost level` the single definition of attribute 2. The measure is the Baa
yield itself, cut at the median of an as-of expanding window (observations up
to and including the one being read, never the future).
`crates/cds-con/src/cost.rs` mirrors `cost_cmd::level_at` byte for byte,
including the integer truncation that decides the label on the boundary — when
the two disagree, finance-cli is right. The present-tense rule is in
[`CLAUDE.md`](CLAUDE.md).

Once the message became the level reading (`render_cost_parts`), the entire
spread renderer beneath it — `render_parts` and everything it called: the
`baa10y`/`baa` lead pair, the per-series 佐證 blocks, the
`cds_message_series` / `cds_message_lead` config plumbing, the
`require_single_kind` adjacency guard — became callerless. It was kept for a
day under a "retired, do not read anything below as delivered" module doc,
then cut: ~518 lines out of `render.rs` and 1878 lines of tests
(`tests/render.rs`) with it. The attribute-2 path (`render_cost_parts`,
`cost.rs`, and the `cost` / `cost_message` / `contract` test files) is
unchanged and green.

**The shape worth keeping.** The baa−aaa derivation lived in `analyze()`,
which the level path still calls — so it was *not* callerless. It ran on
every `analyze()`, computed a `SeriesKind::Spread` row, and was displayed by
nothing after the renderer went: dead-but-reachable, not
dead-and-unreferenced. A shared function can keep a retired feature's
computation alive invisibly, and "no caller" is the wrong test for a function
on a shared path — the right test is whether anything reads its output. The
`BAA_AAA_KEY` / `BAA_AAA_LABEL` / `DERIVED_ORDER` constants and the
`baa_aaa_spread` import went with it, because that derive block was their
only remaining user.

An earlier reverse rule in `CLAUDE.md` forbade any classification in cds-con,
written up from an implementation detail and then cited as policy. It was
never the owner's. The window-flipping objection behind it is answered by the
as-of basis, which fixes one window and prints its `n` — a reading on 129
observations cannot be read as if it were on 1291. The design authority for
the retired shape is `docs/specs/2026-08-04-cds-con-readability-v2-design.md`.

## The degraded alert with no reason, and the ET/UTC split behind it (2026-08-07)

A `cct` eod cron alerted `failure=contract_degraded repair=none` with a body of exactly one string:
`no stderr`. That is nullclaw's literal placeholder (`gateway.zig`, the degraded branch) for a skill
that wrote none. A fix the day before — 58dbcb2, "every rejected envelope has to say why" — was
supposed to have abolished reasonless alerts, and had not.

**It had instrumented one half of a fork.** `api::get` returning `None` warned on every path. The
other arm — payload arrives intact, and the per-mode predicate says it holds no analysis — wrote
nothing at all. Same symptom, other branch. Fixed in 8e92b91 with `content_gap`, which returns `Some`
exactly when the predicate refuses and carries the route's own words as the reason.

**Why the data was missing was not a bug here at all.** GitHub Actions never created cct's 20:05 UTC
end-of-day run on 2026-08-06 — no run exists — after the 16:00 UTC one failed with "The job was not
acquired by Runner of type hosted". The skill was right to degrade; it just would not say so.

**The review found more than the review was for.** An adversarial pass over the fix, corroborated by
hand, turned up three things the fix had not touched:

- The quoted upstream text was unbounded. A hostile `key_events` produced 12,079 bytes of stderr, and
  nullclaw truncates the alert preview at 200 **bytes** — landing mid-codepoint, which hands Telegram
  invalid UTF-8 and can destroy the very alert the commit existed to populate. Bounded in 0bb481b.
- The tests exercised `--mode eod` and nothing else, so suppressing the warning for pre-market alone
  left the whole suite green.
- The report routes derived "today" as `new Date().toISOString().split('T')[0]` — a UTC date — while
  the jobs write under an ET business date. That is the four-to-five-hour window after 00:00 UTC in
  which the reader asks D1 for a day the writer never used.

**That last one turned out to be the real subject.** Chasing it across the cct repo found the same
shape four more times: a `toLocaleString` → `new Date` → `toISOString` round trip that agreed with ET
only because Workers run TZ=UTC; a `normalizeToETDate` that shifts a date-only string back a day; an
`isTradingDay` reading the *host's* weekday, correct on Workers and one day out on the UTC+8
development machine, where it called Sunday a trading day; and `getWeekString`/`getWeekStartDate`
that were not inverses, so on Mondays and Tuesdays the weekly route read the previous week's rows and
labelled them as the current week. Each was individually plausible. None was written down anywhere.

The answer was to stop deriving the day at all. `metadata.business_date` and `metadata.has_content`
now travel on every report envelope: the day the content is *about*, from the row that supplied it,
and whether anything was found for it. "No data for this day" and "here is this day" no longer arrive
in the same shape — which is what let a dead upstream look healthy for 50 days (2026-06-08 to 07-27).

Design: `~/a/cct/docs/specs/2026-08-07-business-date-envelope-design.md`. Consumer plan:
[`docs/superpowers/plans/2026-08-07-cct-consumer-business-date.md`](docs/superpowers/plans/2026-08-07-cct-consumer-business-date.md).
The present-tense rule is in [`CLAUDE.md`](CLAUDE.md).

**Worth keeping from how it went.** Both plans were corrected *during* execution, four times between
them, and every correction was a step that would have produced a working-looking bug: an intraday
content predicate reading a field only the empty shape sets; a weekly clamp writing `2026-W52` into a
`YYYY-MM-DD` field; a clock test that can only fail during four hours of the day. Writing a date rule
without running the code that implements it is the failure this whole entry is about, and the plans
committed it twice before review caught them.

## `lib/`, `scripts/run.py` and autocli — the Python deletion (2026-08-02)

Moved out of `CLAUDE.md` on 2026-08-06. The section had outlived itself in a specific way worth
noting: its heading claimed `lib/` had external dependents while its own first sentence said there
were none left, and it described `~/.nullclaw/skills/lib` in the present tense after that symlink had
been deleted. The rules it still carried (keep `tools/differential/fixtures/`; the sanitizer corpus
is canonical; `run.py:NNN` citations are provenance) stayed in `CLAUDE.md`.

### Related: `lib/` has dependents outside this repo

`~/.nullclaw/skills/lib` symlinks to this repo's `lib/`, and two skills that do
**not** live here resolve their imports through it:

**There are no external consumers left, as of 2026-08-01.** `cct` moved into
this repo and runs Rust; `autocli` was retired.

autocli was removed rather than ported. It had no cron job, had not been
touched since 2026-04-13, and was the only skill not under version control —
a real directory inside `~/.nullclaw/skills` that the local repo there did not
track, with no remote. Its advertised surface did not hold up either: of four
sites tested, only `hackernews top` returned data. `bbc news` and
`reddit frontpage` both failed with "Chrome extension not connected", and
`arxiv paper` failed because the skill passes `--limit` to every subcommand and
that one does not accept it. A copy is at
`~/.nullclaw/skills-archive/autocli.retired.20260801-144629`.

`lib/` and every `scripts/run.py` were deleted on 2026-08-02, along with the
`~/.nullclaw/skills/lib` symlink. Nothing imported them any more: `cct` had
moved into this repo, `autocli` was retired, and `ainews` had been Rust for
weeks — its remaining mentions of `claw-skills/lib` are provenance comments and
an archived script, not imports.

Three things did depend on the Python at deletion time, and were dealt with
first rather than discovered afterwards:

- `crates/{chipcon,inflation-con,oilcon}/tests/differential.rs` each spawned a
  `drive_python.py`. Their verdicts are frozen now; see the oracle table above.
- `tools/differential/*.sh` could not survive the Python and are gone. The
  fixture directory stays, because `crates/weather/tests/sources.rs` reads
  `tools/differential/fixtures/cwa_past_only.json`.
- The sanitizer corpus moved to `claw-core/tests/sanitize_corpus/`, with the
  Python's answers recorded beside it.

Rust source comments still cite `run.py:NNN`. Those are provenance for a rule,
not links — git history is where the line lives now.
