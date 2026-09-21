# autocli — retired 2026-08-01

Kept here because it existed nowhere else. `~/.nullclaw/skills/autocli` was a
real directory that the local repo at `~/.nullclaw/skills` did not track, and
that repo has no remote. Every other skill is a symlink into a versioned repo;
this one was the exception. A signature search across `~/a`, `~/b`,
`.nullclaw` and `.agents` found exactly one copy of `run.py`.

Deleting it would therefore have been irreversible, which is not a property a
removal decision should quietly acquire. There is also a copy at
`~/.nullclaw/skills-archive/autocli.retired.20260801-144629`, but that lives in
the same tree and is equally unversioned.

## Why it was retired rather than ported

No cron job. Untouched since 2026-04-13. Its claim is "structured data from
55+ websites"; of four sites tried, one worked:

    hackernews top      real data
    bbc news            Chrome extension not connected
    reddit frontpage    Chrome extension not connected
    arxiv paper         --limit rejected by that subcommand

The last is the skill's own fault, not the tool's: it appends `--limit` to
every subcommand. That defect would have survived a translation.

## Two things a later reader should not repeat

An outside review corrected two claims made while deciding, and both are worth
recording because they would misinform anyone reviving this:

- **`trace_marker` is not dead code here.** `emit_trace()` is called on both
  success paths (`run.py:190,216`). It is gated on `NULLCLAW_JOB_ID`, not
  unreachable — set the variable and the marker appears.
- **`delivery` is not reached only via `--deliver-to`.** `deliver_or_fail()`
  is called unconditionally; with no target it prints the body to stdout. Only
  `--raw` bypasses it.

So the "just inline the two imports" option that was considered was a larger
change than it looked.

## 2026-08-02

`scripts/run.py` was deleted from this directory when the last Python left the
repo. The `SKILL.md` beside this file still documents commands that need it, so
read it as a record of what the skill was, not as instructions. A complete copy
— script included — is at
`~/.nullclaw/skills-archive/autocli.retired.20260801-144629`, and git history
has it too.
