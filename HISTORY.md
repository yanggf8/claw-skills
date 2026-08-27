# HISTORY.md — retirements, migrations and post-mortems

Narrative **evicted from `CLAUDE.md`**: retired skills, the Python→Rust migration, and "how we found
out" write-ups. `CLAUDE.md` is loaded into every session in this repo, so it keeps only present-tense
rules — current config, live footguns, decision rules. This file is not loaded automatically.

Design specs live in `docs/specs/` and remain the authority for how a thing is *supposed* to work;
this file is only the record of what changed and why.

---

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
