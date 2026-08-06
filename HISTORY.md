# HISTORY.md — retirements, migrations and post-mortems

Narrative **evicted from `CLAUDE.md`**: retired skills, the Python→Rust migration, and "how we found
out" write-ups. `CLAUDE.md` is loaded into every session in this repo, so it keeps only present-tense
rules — current config, live footguns, decision rules. This file is not loaded automatically.

Design specs live in `docs/specs/` and remain the authority for how a thing is *supposed* to work;
this file is only the record of what changed and why.

---

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
