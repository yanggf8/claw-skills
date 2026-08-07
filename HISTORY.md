# HISTORY.md — retirements, migrations and post-mortems

Narrative **evicted from `CLAUDE.md`**: retired skills, the Python→Rust migration, and "how we found
out" write-ups. `CLAUDE.md` is loaded into every session in this repo, so it keeps only present-tense
rules — current config, live footguns, decision rules. This file is not loaded automatically.

Design specs live in `docs/specs/` and remain the authority for how a thing is *supposed* to work;
this file is only the record of what changed and why.

---

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
