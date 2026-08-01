# liko-finance-weekly — Rust port

Live 2026-08-01. `crates/liko-finance-weekly` replaces
`liko-finance-weekly/scripts/run.py` (306 lines). One cron job, 09:00 Sunday
Taipei.

Unlike the other ports in this batch, almost nothing here is computation.
persona-core owns the data and the validator, an agent writes the prose, and
this binary sequences them and reports to the scheduler. The port is therefore
mostly a faithful re-expression, and the interesting part is the small amount
that is not.

## What is actually logic, and is tested

**`next_sunday_taipei`.** The Python added 8 hours to a UTC timestamp and took
the date. That is correct for Taipei, which has had no DST since 1979. It is a
real timezone lookup here anyway: the arithmetic version silently becomes wrong
for any zone that does observe DST, and someone copying it would not be warned.

The case that distinguishes the two: 2026-08-02 20:00 UTC is Monday 04:00 in
Taipei. Read in UTC it is still Sunday and returns 08-02 — an issue dated for a
day that has already ended locally.

**`status_of`.** Matching `id={issue_id} ` with the trailing space, because
`id=iss-002` prefix-matches `id=iss-0021`. The test for this was initially
useless and is worth recording: it looked up the *longer* id, which succeeds
either way. Only looking up the shorter one, against a listing where the longer
appears first, tells the implementations apart. Found by breaking the needle
and watching the suite stay green.

**`body_between`.** The drafting prompt wraps the issue in `BEGIN_ISSUE_BODY` /
`END_ISSUE_BODY` because a model told to research narrates before it writes,
and that narration would otherwise be published. A reply with no markers is
kept whole rather than discarded — the model answered, the validator downstream
decides whether the answer is usable, and dropping it would turn a formatting
slip into a missing week.

## Markers on manual runs: the Python was wrong

Run by hand, the Python emitted `[trace:manual-1785564212]` — a synthetic id it
invented for the occasion. The contract in CLAUDE.md says markers are emitted
"only when `NULLCLAW_JOB_ID` is set, so manual runs stay clean". The Rust emits
nothing without a job id, and both markers with one.

## Deliberately not reproduced: the per-call timeouts

The Python passed `timeout=60`, `120`, `180` to individual persona-core calls.
std::process has no portable per-call timeout, and reproducing those numbers
would mean a thread and a kill per invocation — machinery that looks like a
guarantee. Those values never bounded a run in practice; the scheduler's
`NULLCLAW_SKILL_TIMEOUT` does. The one call that genuinely runs long, the
drafting agent, keeps its explicit bound via `--agent-timeout` (900s default).

## Kept

- One repair pass, not a loop. A validator that rejects the same body twice is
  reporting something the model cannot fix by trying harder.
- `published` and `delivered` mean the run is a no-op; `skipped` does not. A
  week that failed validation should be retried, not treated as finished.
- History is best-effort: an unreadable archive produces
  `(history unavailable: …)` in the context rather than losing the week.
- Both prompts byte for byte. They encode an editorial contract — three
  mandatory headings in a fixed order, a verb whitelist for the action list, a
  ban on trading language — and persona-core's R1/R2/R3 validator enforces the
  same rules from the other side. Rewording either would desynchronise them.
- Failure is reported as `[skill-status:failed]` with exit 0, not a non-zero
  exit: nullclaw's exit_code branch overrides marker parsing, and the marker is
  the more precise signal. Nothing is published on failure, so there is no
  delivery to suppress.

## Tests: 17

Verified by breaking things and checking the right test went red. Two of the
first three breakages were not caught, and both were instructive: one was a
`sed` that never applied — the harness lying, not the suite failing — and the
other was the genuinely weak prefix test described above.
