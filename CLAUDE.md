# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this repo is

Personal agent skills invoked as cron jobs or on-demand by the **nullclaw**, **openclaw**, or **nanoclaw** agent. Each skill lives in its own directory. Same source, same `SKILL.md` format, three hosts.

**The port from Python to Rust finished on 2026-08-02.** The standing instruction was that the whole stack is Rust, and it now is: every skill runs a binary built from `crates/`, and no Python remains except `docs/superpowers/seed.py`, which is a persona-webapp DB seed/restore tool rather than a skill and has no `persona-core` equivalent.

- `crates/` — a Cargo workspace holding one binary crate per ported skill. **The shared `claw-core` lives in `../../b/gwebcdb/crates/claw-core`**, consumed by path dependency — gwebcdb is this ecosystem's home for cross-repo Rust crates (the same arrangement as `turso-util` → `finance-cli`), and any future cross-repo consumer will need claw-core the same way. Building claw-skills therefore requires gwebcdb checked out beside it. Cutover is one line — the `## Script` path in `SKILL.md` — but **publish the binary with `tools/install-skill.sh <skill>`, never by hand**. That script builds `--locked`, stages, smoke-probes the artifact with an unknown flag and requires **exit 2**, publishes atomically, and verifies the path nullclaw will resolve. A manual `install` bypasses the probe, and on 2026-07-31 that probe caught a real defect in oilcon: its argument parser silently ignored unknown flags and accepted any `--mode` value, so a typo would have run the record branch — which never delivers — while still reporting `[skill-status:ok]`.

**The oracle outlived the Python.** Every port was checked by running both
implementations over the same inputs, and that is what caught the real bugs —
never the test suites, which were green throughout. Deleting the Python would
have thrown that away, so its verdicts were recorded first and committed:

| Frozen oracle | What it pins |
|---|---|
| `crates/{chipcon,inflation-con,oilcon}/fixtures/python-oracle.txt` | status, rendered message and history line per fixture set |
| `claw-core/tests/sanitize_corpus/expected.json` | 28 corpus inputs × both modes |

The comparisons in `differential.rs` are byte-for-byte and unchanged; they read
the verdict from disk instead of re-deriving it. Regenerating any of them means
resurrecting the Python from git history, which is deliberate — these are
answers, not a program, and they should change only when someone decides they
should. Emptying one fails its test rather than passing vacuously.

The port is finished, but `docs/specs/2026-07-28-phase1-lessons.md` is still the first thing to read before writing tests here: it records the anti-patterns that produced a fully green suite protecting nothing, and they kept recurring right through the last skill. Porting history is in `docs/superpowers/plans/`.

## Current agent support

All three agents are supported by the same code. The `SKILL.md` format is the standard Claude Code skill format — all three use the same frontmatter (`name`, `description`, `always`). Differences are isolated to config/env resolution and install layout:

| Concern                  | nullclaw                                               | openclaw                                                                    | nanoclaw                                                      |
|--------------------------|--------------------------------------------------------|-----------------------------------------------------------------------------|---------------------------------------------------------------|
| Agent CLI                | `nullclaw ...`                                         | `openclaw ...`                                                              | `nanoclaw ...`                                                |
| Config file (JSON)       | `~/.nullclaw/config.json` (default)                    | `~/.openclaw/openclaw.json` — set via `CLAW_CONFIG` env var                 | set via `CLAW_CONFIG` env var                                 |
| Env file (dotenv)        | `~/.nullclaw/.env` (default)                           | typically `~/.openclaw/.env` — set via `CLAW_ENV` env var                   | set via `CLAW_ENV` env var                                    |
| Telegram config shape    | `channels.telegram.accounts.<name>.bot_token`          | `channels.telegram.botToken` (single token)                                 | auto-detected (same as nullclaw or openclaw)                  |
| `--account` flag         | Selects account in multi-account config                | No-op (openclaw is single-token)                                            | same as nullclaw or openclaw depending on config              |
| Install location         | Symlink each skill to `~/.nullclaw/skills/<name>`      | Repo **is** the skills dir at `<workspace>/skills/` (typically `~/clawd/skills/`) | Symlink each skill to `<nanoclaw>/container/skills/<name>`    |
| Skill discovery CLI      | `nullclaw skills list`                                 | `openclaw skills list` (source column shows `openclaw-workspace`)           | loaded by container agent at runtime                          |
| Cron scheduling          | `nullclaw cron add-skill ...`                          | `openclaw cron ...`                                                         | `nanoclaw cron ...`                                           |
| Memory / agent invoke    | `nullclaw agent -m "<prompt>"`                         | `openclaw agent -m "<prompt>"` (see openclaw docs)                          | via nanoclaw container agent                                  |
| Runtime requirement      | none (static binary)                                   | none (static binary)                                                        | build the binaries for the container's architecture           |

`claw-core`'s telegram module auto-detects the schema: it tries the nullclaw multi-account path first and falls back to openclaw's `botToken`. A single install can target both configs by switching `CLAW_CONFIG` — the binary does not need to know which host it is running under.

### OpenClaw-specific constraint

OpenClaw's skill loader (`src/agents/skills/workspace.ts`) calls `realpath` on every candidate path and rejects anything whose real path is not inside `<workspace>/skills/`. Consequence:

- Symlinks into `<workspace>/skills/` from a sibling dir (e.g. `~/clawd/external-skills/`) are **silently ignored** — you will see `[skills] Skipping skill path that resolves outside its configured root.` warnings.
- The working layout is to keep this git repo directly at `<workspace>/skills/`. Non-skill entries at the repo root (`README.md`, `CLAUDE.md`, `crates/`, `.git/`) are harmless — the loader only loads immediate subdirs that contain a `SKILL.md`.

### Nullclaw-specific notes

- Multi-account Telegram is supported; pass `--account <name>`.
- `~/nullclaw/zig-out/bin/nullclaw` is the assumed binary path for skills that shell out to the agent (weather's clothing advice, news, mindfulness-spirit). On an openclaw-only host that spawn fails, and each caller logs a warning and continues rather than failing the run.

## Host layout

- **nullclaw**: each skill symlinked into `~/.nullclaw/skills/<name>`. Repo may live anywhere.
- **openclaw**: the repo itself is the workspace skills dir (`<workspace>/skills/`). OpenClaw's loader `realpath`s every candidate and rejects anything resolving outside the skills root, so sibling-dir symlinks do not work. Dotfiles and dirs without `SKILL.md` at the repo root (`crates/`, `README.md`, `.git`) are ignored by the loader.
- **nanoclaw**: each skill symlinked into `<nanoclaw>/container/skills/<name>`. The container agent discovers `bin/<name>` relative to `SKILL.md`. The binaries are self-contained, so the container needs no interpreter — but they must be built for its architecture.

## Skill structure

Every skill directory contains:
- `SKILL.md` — frontmatter (`name`, `description`, `always: true`) + usage docs. All three agents read this.
- `bin/<name>` — the published binary, gitignored. Put it there with
  `tools/install-skill.sh <name>`, never by hand.

The source lives in `crates/<name>/`. Shared behaviour — delivery, the
scheduler markers, env and config resolution, the bounded HTTP agent, the
nested-agent sanitizer — comes from `claw-core`:

```rust
use claw_core::delivery::{deliver, DeliverOptions};   // canonical delivery path
use claw_core::outcome::{finish, Finish};             // exit code + markers
use claw_core::http::agent;                           // a timeout that bounds connect
```

## Config / env resolution

Scripts resolve config and env files in this order:

1. `$CLAW_CONFIG` env var → JSON config path
2. `$CLAW_ENV` env var → dotenv path
3. Defaults: `~/.nullclaw/config.json`, `~/.nullclaw/.env`

Export the env vars once per host (e.g. in `~/.profile`):

```bash
# openclaw
export CLAW_CONFIG="$HOME/.openclaw/openclaw.json"
export CLAW_ENV="$HOME/.openclaw/.env"           # optional, only if you keep API keys here
```

`claw-core`'s telegram module auto-detects the schema:

- **nullclaw**: `channels.telegram.accounts.<account>.bot_token`
- **openclaw**: `channels.telegram.botToken` (single token — `--account` is a no-op)

## Running skills

```bash
# nullclaw
~/.nullclaw/skills/<name>/bin/<name> [options]

# openclaw
~/clawd/skills/<name>/bin/<name> [options]

# Examples
~/clawd/skills/stock/bin/stock --market tw
~/clawd/skills/news/bin/news --deliver-to 7972814626
~/clawd/skills/weather/bin/weather --location 臺北市
```

## Telegram delivery

`claw_core::delivery::deliver(chat_id, body, &opts, out, err)` is the canonical
path; it prints the body to stdout when `chat_id` is absent and, on a send
failure, prints it anyway so the cron capture keeps the data.

Most skills accept `--deliver-to CHAT_ID` and `--account NAME`. Two do not:
`mindfulness-spirit` routes by the persona-core column's `delivery_target`, and
`commute` delivers through `traffic`. Omitting `--deliver-to` sends output to
stdout, which is how a cron job is debugged by hand.

## Registering with the agent

**nullclaw**:
```bash
ln -s ~/claw/claw-skills/<skill> ~/.nullclaw/skills/<skill>
nullclaw skills list
```

**openclaw**:
```bash
# Repo is already at ~/clawd/skills/ — loader picks it up automatically.
cd ~/clawd && openclaw skills list    # expect source=openclaw-workspace
```

**nanoclaw**:
```bash
ln -s ~/claw/claw-skills/<skill> ~/claw/nanoclaw/container/skills/<skill>
# Loaded by the container agent at runtime — no separate list command needed.
```

## SKILL.md frontmatter

```yaml
---
name: skill-name
description: One-line description shown in the agent's skill list
always: true        # load into agent context automatically
---
```

The `## Script` section hints what cron should run for `job_type=skill` jobs. The `## Prompt` section (if present) is used when the skill is invoked interactively by the LLM.

Note: existing `## Script` paths reference `~/.nullclaw/skills/...` — that's documentation for the nullclaw host. OpenClaw and nanoclaw both discover the binary relative to the `SKILL.md` location and ignore the literal path.

## Adding a new skill

1. Create `<skill>/SKILL.md` and `crates/<skill>/`, and add the crate to the
   workspace `members`
2. The binary must: reject an unknown flag with **exit 2** (the install probe
   requires it); deliver via `claw_core::delivery::deliver`; exit the process
   through `claw_core::outcome::finish` so the markers and the exit code stay
   consistent; and treat an upstream failure as a degraded delivery rather than
   a crash
3. Publish with `tools/install-skill.sh <skill>`
4. For nullclaw: `ln -s ~/claw/claw-skills/<skill> ~/.nullclaw/skills/<skill>`
5. For openclaw: already discovered if the repo is at `<workspace>/skills/`
6. For nanoclaw: `ln -s ~/claw/claw-skills/<skill> ~/claw/nanoclaw/container/skills/<skill>`
7. Verify with `nullclaw skills list` or `openclaw skills list`

## Cron scheduling

**nullclaw**:
```bash
nullclaw cron add-skill "35 13 * * 1-5" <skill> --deliver-to <chat_id> --skill-args "<args>"
nullclaw cron list
nullclaw cron backup
```

**openclaw**: use `openclaw cron` (see `openclaw cron --help`).

**nanoclaw**: use `nanoclaw cron` (see nanoclaw docs).

Cron expressions use UTC. Taiwan (CST) = UTC+8, EST = UTC-5.

## Scheduler contract (hard constraints)

Three rules any skill must satisfy, in **any** language. They bind a rewrite —
Rust, Go, whatever — exactly as tightly as they bind the current Python.

### 1. The stdout markers are matched literally

`nullclaw`'s `src/cron.zig` → `classifySkillRun()` parses two exact lines out of
stdout. Not a regex over prose, not JSON — literal marker lines:

```
[skill-status:ok|degraded|failed]
[trace:<job id>]
```

Classification, straight from that function:

| stdout | `failure_class` | `verified` |
|--------|-----------------|------------|
| both markers, status `ok` | — | 1 |
| both markers, status `degraded` | `contract_degraded` | 2 |
| both markers, status `failed` | `contract_failed` | 3 |
| `[trace:]` present, no status line | `contract_missing` | 2 |
| no `[trace:]` at all | `content_invalid` | 2 |
| non-zero exit | `exec_error` | 3 |
| timeout | `timeout` | 3 |

Emit them **after** delivery is confirmed, and only when `NULLCLAW_JOB_ID` is
set, so manual runs stay clean. `NULLCLAW_JOB_ID` is the per-**run** trace id, and
`classifySkillRun` compares the `[trace:]` payload to it byte for byte — read it
fresh each run, never cache it. Anything else on stdout is the message body —
keep diagnostics on stderr. `verified` is not a boolean: `0=unverified 1=ok
2=degraded 3=failed_verify`.

### 1b. Scheduled skills now receive a delivery budget (since 2026-07-28)

`NULLCLAW_SKILL_TIMEOUT` and `NULLCLAW_SKILL_STARTED` are set on **scheduled**
spawns as well as manual ones (nullclaw `35bca969`). Before that only the manual
path set the timeout and `NULLCLAW_SKILL_STARTED` existed nowhere, so
`delivery.py`'s elapsed-time branch had never executed in production and every
scheduled delivery fell back to telegram's flat 30s cap.

`NULLCLAW_SKILL_STARTED` is **CLOCK_MONOTONIC seconds** — the same clock domain
as `time.monotonic()`. A wall-clock value makes `now - started` hugely negative,
which `max(0.0, ...)` clamps to zero, so the budget silently does nothing while
looking perfectly healthy. A job with no explicit timeout exports `0`, which
`delivery.py`'s `if timeout <= 0` turns back into the 30s default.

Net effect: telegram is capped at 3 attempts with 2s+5s backoff, so wall time
stays under ~52s regardless of budget. The budget only tightens that bound; it
never adds attempts.

### 2. Every cron job now runs `skill_contract` — there is no lax mode left

All 38 jobs use `verification_mode = skill_contract`
(`weather` 8, `traffic` 7, `news` 6, `cct` 4, `doughcon` 4, `cct2` 2,
`ainews` 2, one each for the rest). A skill that gets the markers wrong alerts
the same day.

This is new. The four `cct` jobs sat on `verification_mode = none` until
2026-07-27, which passes unconditionally — that is why a dead upstream pipeline
delivered stale reports for 50 days without a single alert. The buffer is gone,
which is the point: mistakes are now loud instead of silent.

### 3. `retry_once` + deliver-before-classify = duplicate Telegram messages

Observed 2026-07-29: `cct2 --mode pre-market` delivered two messages one minute
apart, both carrying the same trace id `…:3818` — one run, retried in place:

```
run#1540  13:35:12 → 13:36:30
          failure_class=contract_failed  repair_action=retried_ok  verified=1
```

The scheduler retries whenever a run ends `verified != 1` and the job is on
`repair_policy = retry_once` (`cron.zig:5622`). The retry re-execs with
**`retry_child.env_map = &skill_env`** — the same environment, so
`NULLCLAW_JOB_ID` is byte-identical. **A skill cannot tell it is the retry.**
There is no attempt counter to branch on.

Meanwhile skills deliver *before* they classify, because that is what this file
prescribes ("call only after delivery confirmation" — correct for marker
accuracy). **The Rust port inherits the pattern rather than fixing it**, so this
is a live defect in ported code, not a Python-only legacy:

| Impl | Skills | Evidence |
|------|--------|----------|
| Rust (live) | `weather`, `doughcon`, `traffic` | `weather/src/main.rs:116` deliver → `:129` `Finish::Marked`; `doughcon/src/main.rs:108` → `:113`. `traffic` sidesteps it: its degraded path exits before delivery, and its jobs are on `repair_policy = none` because no traffic failure is repaired by a retry. |
| Rust (live) | `news` | Satisfies option A rather than inheriting the defect: both hard-failure paths (`all_feeds_empty`, `ai_exhausted`) alert and exit 1 without delivering, and the skill never emits `degraded` at all — a section that falls back still ships as `ok`. One duplicate window remains, inherited from the Python: a long digest is sent as several chunks in sequence, so a failure on chunk 2 exits 1 with chunk 1 already delivered, and the retry re-sends it. |

So any first attempt that delivers successfully and then reports `degraded` or
`failed` has already put a message in front of the user when the retry fires.
34 of 38 jobs are on `retry_once`. Prior occurrences: oilcon 2026-07-20,
cct2 eod 2026-07-10, chipcon 2026-07-08 (×2).

**Decision for the Rust port — option A: the hard-failure path must not
deliver.** When a skill has no usable result, emit `[skill-status:failed]` and
`[trace:]` and send nothing. The retry then becomes the only thing that can
deliver, so a rescued run produces exactly one message — on 2026-07-29 that
would have been the real report alone, with no "⚠️ 無法取得任何分析結果" noise.
If both attempts fail the user gets no Telegram message at all; that is
intended, because the cron alert is the right channel for "the skill produced
nothing", not a report body.

Reference implementation: `crates/cct2/src/main.rs` and its delivery-target
branch. Reuse it rather than reinventing the rule; the remaining
deliver-then-mark skills still need it.

Still open, and **not** solved by option A: `degraded` runs are *meant* to
deliver (a stale-but-real report still has value — see `cct` pre-market), yet
`degraded` is also `verified != 1`, so it triggers the same retry and the same
duplicate. Either drop `retry_once` from jobs whose skills deliver on
`degraded`, or restrict scheduler retries to `failed` / `exec_error` /
`timeout` — the latter is a nullclaw (Zig) change, not a skills change. Note a
retry cannot repair `degraded` anyway: stale or empty upstream data returns
identical on the second attempt.

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

## HTTP timeouts (hard constraint)

Every HTTP call goes through `claw_core::http::agent(timeout)`. Nothing builds
its own `ureq` agent, and `tools/lint-http.sh` fails the tree if anything
starts to.

`ureq`'s own `timeout` — on the builder **or** on the request — does not bound
the connect phase. Only `timeout_connect` does. A host that *refuses* a
connection fails instantly either way, which is why this looked fine
everywhere; a host that drops packets hangs for ureq's 30-second connect
default no matter what the caller asked for. Measured 2026-08-01 against an
address routed nowhere, with a 50 ms budget:

| how the timeout was set                  | elapsed |
|------------------------------------------|---------|
| `ureq::builder().timeout(d)`             | 30.03 s |
| `ureq::get(u).timeout(d)`                | 30.02 s |
| `timeout_connect` + `timeout_read/write` | 50.8 ms |

Thirteen call sites had it: all twelve skills plus `claw-core`'s Telegram send,
whose documented "~52s across three attempts" was really ~97s whenever
api.telegram.org was unreachable rather than slow. Every one of them is a
**port regression** — Python's `urlopen(req, timeout=N)` covers the connect, so
each skill silently lost a bound it used to have on the way to Rust. No output
differential can see it: the difference is time, not bytes.

Three things hold the property now, and they cover different failures:
`claw-core`'s unit tests prove the wrapper bounds a hanging connect;
`tools/lint-http.sh` proves every crate uses the wrapper; and
`crates/doughcon/tests/cli.rs` measures one crate end to end, because a static
check cannot see composition. That last one costs ~20s — the crate's own budget
— and is worth it.

## Testing

```bash
cargo test --workspace                 # offline, no interpreter, no API key
cargo clippy --workspace --all-targets # expected: zero warnings
tools/lint-http.sh                     # every HTTP call bounded
```

`claw-core` lives in another repo and is not in this workspace:

```bash
cd ../../b/gwebcdb && cargo test -p claw-core
```

The suites that look like they would need something — the differentials, the
sanitizer corpus, the delivery and pipeline tests — read recorded fixtures or
drive a local stub.

## Gotchas

- **`weather` needs `CWA_API_KEY`**: put it in `~/.nullclaw/.env` (default) or `~/.openclaw/.env` and export `CLAW_ENV` to point at it. Without the key, Taiwan forecasts silently return no data.
- **OpenClaw `weather` name collision**: OpenClaw ships a bundled `weather` skill (wttr.in). Workspace skills take precedence, so this repo's `weather` wins. Rename the folder + frontmatter `name:` if you want both.
- **`oilcon` no longer needs `libsql-experimental`**: the Rust build links libsql directly, so the PEP 668 pip dance Ubuntu 24.04 used to require is gone and `oilcon/requirements.txt` is dead. A `WARN: turso unavailable` from oilcon now means credentials or reachability, not a missing wheel.

## Design notes

Prior design context lives in `docs/specs/` (`2026-04-15-oilcon-skill-design.md`, `2026-04-16-turso-consolidation.md`, `2026-04-18-persona-webapp-reconciliation.md`, `oil-trend-rule.md`). Check there before redesigning a skill from scratch.

## Skills reference

| Skill | Script args | External API |
|-------|-------------|--------------|
| `news` | `--topics`, `--account-topics`, `manage list\|add\|remove` | Google News RSS |
| `cct` | `--mode pre-market\|intraday\|eod\|weekly` | CCT internal |
| `cct2` | `--mode pre-market\|eod` | Yahoo Finance + dual LLM, both over direct HTTPS |
| `stock` | `--market tw\|hk\|all`, `--symbol CODE` | TWSE, Yahoo Finance |
| `chipcon` | `--mode record`, `--deliver-to` | Yahoo Finance chart (SMH/QQQ/SOXX); observation-only report |
| `weather` | `--location NAME` (repeatable) | CWA (Taiwan), HKO (HK) |
| `traffic` | `--from`, `--to`, `--via` | TomTom Routing API |
| `doughcon` | `--mode deliver\|record`, `--et-hour H` (DST gate) | PizzINT API |
| `oilcon` | `--mode deliver\|record` | Yahoo Finance, Turso |
| `agent-reach` | agent-only, see SKILL.md | 13+ platforms |
| `mindfulness-spirit` | `write`, `fix-signature DEVTO_ID`, `--dry-run` | Google News RSS, Turso + delivery via `persona-core` CLI |
| `liko-finance-weekly` | `--dry-run`, `--check` | Turso (via `persona-core` CLI) |

`persona-skill` was retired (Step 10 of the persona-core absorbing
plan). Persona CRUD, secrets, history, and editorial plans are now
managed exclusively via the `persona-core` Rust CLI:

```bash
persona-core personas show <slug>
persona-core personas list
persona-core secrets set <slug> <kind> <value>
persona-core history list --persona <slug>
persona-core plans list
```

### Shared crate (`claw-core`)

Lives at `../../b/gwebcdb/crates/claw-core` and is consumed by path dependency,
so building this repo needs gwebcdb checked out beside it.

| Module | Purpose |
|--------|---------|
| `delivery` | **Canonical** delivery: `deliver(chat_id, body, &opts, out, err)`. Prints the body to stdout when there is no chat id, and prints it anyway on a send failure so the cron capture keeps the data. Returns an outcome; it never exits the process, because exit code and semantic status are independent in nullclaw's classification. |
| `telegram` | Bounded-retry send. Auto-detects the nullclaw/openclaw config schema. |
| `marker` / `outcome` | Scheduler markers and the exit path. `finish(Finish::Marked{..})` emits both lines; `finish(Finish::Unmarked{exit})` emits neither, which is right for any non-zero exit since nullclaw's exit-code branch wins. No-ops when `NULLCLAW_JOB_ID` is unset, so manual runs stay clean. |
| `http` | `agent(timeout)` — the **only** way a skill may build an HTTP client. See the hard constraint below. |
| `sanitize` | `strip_agent_artifacts()` — strips `<ncchoices>` blocks (paired *and* unclosed) and harness marker lines from nested-agent stdout. Any skill that delivers raw `nullclaw agent` output must run it through this. `collapse_blank_lines=false` is the markdown-safe mode for article bodies. |
| `agent` | `call_agent()` for the simple "ask for advice, empty on failure" case. Skills that must branch on the exit code — news, mindfulness-spirit — own their own runner instead. |
| `config` / `env` | `~/.nullclaw/config.json` and dotenv resolution, honouring `CLAW_CONFIG` / `CLAW_ENV`. |
| `budget` | `monotonic_secs()` and the delivery deadline from `NULLCLAW_SKILL_TIMEOUT` / `NULLCLAW_SKILL_STARTED`. |

Skill-specific helpers that used to live in `lib/` now sit in the crate that
owns them: `oil_fetch` / `oil_store` in `crates/oilcon`, `news_quality` in
`crates/news/src/quality.rs`.

`cover_image` (CogView-4 covers) and `heartbeat` had no caller left and were
deleted with the rest. `persona_registry` / `persona_history` were retired
earlier alongside `persona-skill`; persona access goes through the
`persona-core` CLI.
