# cct — moved here, and ported to Rust

Live 2026-08-01. `crates/cct` replaces `cct/scripts/run.py` (561 lines,
29 tests). Four cron jobs: pre-market, intraday, eod, weekly.

## It moved repos first

The skill lived in `~/a/cct/skills/cct` — the Cloudflare Worker's own repo —
and imported `delivery` and `trace_marker` from `~/a/claw-skills/lib`. The
dependency ran backwards: a Worker repo reaching into an agent-skill library
for its runtime contract. The skill reads the CCT API and delivers to Telegram;
that is an agent skill, and it now sits with the other fourteen.

`~/.nullclaw/skills/cct` was repointed. The four cron jobs address the skill by
name, so they were untouched.

## Why every mode has its own content predicate

The CCT API answers 200 with `success: true` even when a job never ran or
outright failed — the route turns a `status === 'failed'` job into a success
envelope carrying only a message. Deciding ok-vs-degraded on "did a payload
arrive" reports ok while the pipeline is broken, which is exactly what happened
for 50 days from 2026-06-08.

Each empty state has a different shape, so each mode tests for real content:
pre-market and intraday zero their counters, eod zeroes a nested one and
carries no message at all, weekly loses `report` entirely.

## The three incidents these tests hold shut

All three are real, all three are now covered, and breaking each one turns the
right tests red:

| break | red |
|---|---|
| trust `is_stale` alone, stop comparing dates | 3 |
| let a stale report list its signals again | 3 |
| test only `daily_summary` for eod content | 2 |

The third is the 2026-07-21 regression: the live payload is a flat camelCase
scorecard and carries no `daily_summary`, which only ever appears in the
placeholder, so testing for it alone reported degraded on every genuine report.

## Found by differential, not by tests

`overall_sentiment`, not `market_sentiment`. The first version of this port
read the wrong key from `daily_summary` and silently dropped the 今日總結 line
from every placeholder report. No unit test caught it — a diff against the
Python on the live payload did.

Worse, the test written alongside it used the same wrong key, so it passed
against the broken code and would have passed forever. It had been written from
the implementation instead of from the payload. Both the code and the test are
fixed, and the test now says why it exists.

The eod branch is also chosen explicitly now — `signalBreakdown ||
totalSignals || modelGrade` — rather than by "did the scorecard renderer
produce any lines". That heuristic picks the wrong branch for a scorecard thin
enough to render nothing, and would then print a summary the payload never had.

## Tests: 32

The 29 Python cases are carried over, plus three for the field-name and
dispatch faults above. Two fixtures are real: the pre-market payload captured
on a day the route served a 50-day-old snapshot, and a genuine eod scorecard.

Differential against the live API: all four modes byte-identical.
