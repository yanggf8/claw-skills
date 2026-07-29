# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this repo is

Personal agent skills invoked as cron jobs or on-demand by the **nullclaw**, **openclaw**, or **nanoclaw** agent. Each skill lives in its own directory. Same source, same `SKILL.md` format, three hosts.

**The repo is mid-port from Python to Rust** (standing instruction: the whole stack is Rust). Today it is both:

- `lib/` + `<skill>/scripts/run.py` — the Python that is **live**. Every cron job still runs it.
- `crates/` — a Cargo workspace holding one binary crate per ported skill. **The shared `claw-core` lives in `../../b/gwebcdb/crates/claw-core`**, consumed by path dependency — gwebcdb is this ecosystem's home for cross-repo Rust crates (the same arrangement as `turso-util` → `finance-cli`), and `cct` / `autocli` will need claw-core too once they port. Building claw-skills therefore requires gwebcdb checked out beside it. **`doughcon` is ported but NOT live** — its `SKILL.md` still points at the Python; cutover is one line.

**The Python `lib/` cannot be deleted when a skill is ported.** `cct` (`~/a/cct/skills/cct`) and `autocli` (`~/.nullclaw/skills/autocli`) import `delivery` and `trace_marker` from it from **outside this repo**. Both implementations coexist until every consumer moves.

Porting sequence and status: `docs/superpowers/plans/2026-07-28-con-family-rust-port-phase1.md` (Phase ① done). **Before porting anything else, read `docs/specs/2026-07-28-phase1-lessons.md`** — it records the test anti-patterns that produced a fully green suite protecting nothing, the Python-vs-Rust semantic traps, and toolchain facts verified by compiling rather than assuming.

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
| Python requirement       | host python3                                           | host python3                                                                | python3 must be installed in the container Dockerfile         |

`lib/telegram.py` auto-detects schema: tries the nullclaw multi-account path first, falls back to openclaw's `botToken`. This means a single install can even target both configs at once by switching `CLAW_CONFIG` — the lib does not need to know which host it is running under.

### OpenClaw-specific constraint

OpenClaw's skill loader (`src/agents/skills/workspace.ts`) calls `realpath` on every candidate path and rejects anything whose real path is not inside `<workspace>/skills/`. Consequence:

- Symlinks into `<workspace>/skills/` from a sibling dir (e.g. `~/clawd/external-skills/`) are **silently ignored** — you will see `[skills] Skipping skill path that resolves outside its configured root.` warnings.
- The working layout is to keep this git repo directly at `<workspace>/skills/`. Non-skill entries at the repo root (`README.md`, `CLAUDE.md`, `lib/`, `.git/`) are harmless — the loader only loads immediate subdirs that contain a `SKILL.md`.

### Nullclaw-specific notes

- Multi-account Telegram is supported; pass `--account <name>` when calling `run.py`.
- `~/nullclaw/zig-out/bin/nullclaw` is the assumed binary path for skills that shell out to the agent (e.g. `weather/scripts/run.py`'s clothing-advice prompt). On an openclaw-only host that subprocess call will fail and the script logs `[WARN]` and continues.

## Host layout

- **nullclaw**: each skill symlinked into `~/.nullclaw/skills/<name>`. Repo may live anywhere.
- **openclaw**: the repo itself is the workspace skills dir (`<workspace>/skills/`). OpenClaw's loader `realpath`s every candidate and rejects anything resolving outside the skills root, so sibling-dir symlinks do not work. Dotfiles and dirs without `SKILL.md` at the repo root (`lib/`, `README.md`, `.git`) are ignored by the loader.
- **nanoclaw**: each skill symlinked into `<nanoclaw>/container/skills/<name>`. The container agent discovers `scripts/run.py` relative to `SKILL.md`. Requires `python3` in the container.

## Skill structure

Every skill directory contains:
- `SKILL.md` — frontmatter (`name`, `description`, `always: true`) + usage docs. Both agents read this.
- `scripts/run.py` — the executable. Always exits 0 (prints `[WARN: ...]` on failure instead of raising).

The `lib/` directory is a shared Python package, not a skill. All scripts add it to
`sys.path` via the same relative pattern, then import what they need. The canonical
delivery import is `deliver_or_fail` from `delivery` (`telegram` is also importable
for direct use):
```python
SKILLS_LIB = os.path.join(os.path.dirname(__file__), "..", "..", "lib")
sys.path.insert(0, os.path.abspath(SKILLS_LIB))
from delivery import deliver_or_fail   # canonical delivery path
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

`lib/telegram.py` auto-detects schema:

- **nullclaw**: `channels.telegram.accounts.<account>.bot_token`
- **openclaw**: `channels.telegram.botToken` (single token — `--account` is a no-op)

## Running skills

```bash
# nullclaw
python3 ~/.nullclaw/skills/<name>/scripts/run.py [options]

# openclaw (scripts resolve their own lib via relative path)
python3 ~/clawd/skills/<name>/scripts/run.py [options]

# Examples
python3 ~/clawd/skills/stock/scripts/run.py --market tw
python3 ~/clawd/skills/news/scripts/run.py --deliver-to 7972814626
python3 ~/clawd/skills/weather/scripts/run.py --location 臺北市
```

## Telegram delivery

`lib/telegram.send(chat_id, text, account="main", config_path=None)` sends a message. `config_path` overrides the resolution order above.

All skill `run.py`s accept `--deliver-to CHAT_ID` and `--account NAME`. When `--deliver-to` is omitted, output goes to stdout (useful for cron debugging).

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

Note: existing `## Script` paths reference `~/.nullclaw/skills/...` — that's documentation for the nullclaw host. OpenClaw and nanoclaw both discover scripts relative to the `SKILL.md` location and ignore the literal path.

## Adding a new skill

1. Create `<skill>/SKILL.md` and `<skill>/scripts/run.py`
2. Script must: accept `--deliver-to` and `--account`; deliver via `deliver_or_fail` from `lib/delivery.py` (echoes body to stdout when `--deliver-to` is omitted, and on send failure echoes the body + `exit(1)` so cron capture keeps the data); exit 0 on upstream API/fetch errors (print `[WARN: ...]` instead of raising)
3. For nullclaw: `ln -s ~/claw/claw-skills/<skill> ~/.nullclaw/skills/<skill>`
4. For openclaw: already discovered if the repo is at `<workspace>/skills/`
5. For nanoclaw: `ln -s ~/claw/claw-skills/<skill> ~/claw/nanoclaw/container/skills/<skill>`
6. Verify with `nullclaw skills list` or `openclaw skills list`

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

Two rules any skill must satisfy, in **any** language. They bind a rewrite —
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
(`weather` 8, `commute` 7, `news` 6, `cct` 4, `doughcon` 4, `cct2` 2,
`ainews` 2, one each for the rest). A skill that gets the markers wrong alerts
the same day.

This is new. The four `cct` jobs sat on `verification_mode = none` until
2026-07-27, which passes unconditionally — that is why a dead upstream pipeline
delivered stale reports for 50 days without a single alert. The buffer is gone,
which is the point: mistakes are now loud instead of silent.

### Related: `lib/` has dependents outside this repo

`~/.nullclaw/skills/lib` symlinks to this repo's `lib/`, and two skills that do
**not** live here resolve their imports through it:

| Skill | Real location | Imports |
|-------|---------------|---------|
| `cct` | `~/a/cct/skills/cct` | `delivery`, `trace_marker`, via `_resolve_skills_lib()` → `../../lib` |
| `autocli` | `~/.nullclaw/skills/autocli` (a real dir, not a symlink) | same |

Removing or porting Python `lib/` breaks both at import time — non-zero exit,
so cron records `exec_error`. Decide the compatibility story (keep a Python
shim / move those skills too / vendor a copy) *before* the port, not after a
cron finds out.

## Testing

`lib/test_*.py` are plain `unittest` files, runnable directly with no pytest or runner:

```bash
python3 lib/test_telegram_retry.py
python3 lib/test_oil_store.py
# etc.
```

## Gotchas

- **`weather` needs `CWA_API_KEY`**: put it in `~/.nullclaw/.env` (default) or `~/.openclaw/.env` and export `CLAW_ENV` to point at it. Without the key, Taiwan forecasts silently return no data.
- **OpenClaw `weather` name collision**: OpenClaw ships a bundled `weather` skill (wttr.in). Workspace skills take precedence, so this repo's `weather` wins. Rename the folder + frontmatter `name:` if you want both.
- **`oilcon` needs `libsql-experimental` on the host python**: declared in `oilcon/requirements.txt` and imported by `lib/oil_store.py`. If missing, oilcon degrades (`contract_degraded`) and delivers `WARN: turso unavailable - libsql-experimental not installed` — TURSO creds being present does not help. On an externally-managed host python (PEP 668, e.g. Ubuntu 24.04 `/usr/bin/python3`), both plain and `--user` pip are blocked; install with `python3 -m pip install --user --break-system-packages libsql-experimental` (lands in `~/.local`, cp312 manylinux wheel, no Rust build; `--user` keeps it out of system site, `--break-system-packages` only clears the PEP 668 gate). Cron runs bare `/usr/bin/python3`, which resolves `~/.local` user-site via `HOME`, so it picks it up. Fixes any Python skill importing `libsql_experimental`, not just oilcon.

## Design notes

Prior design context lives in `docs/specs/` (`2026-04-15-oilcon-skill-design.md`, `2026-04-16-turso-consolidation.md`, `2026-04-18-persona-webapp-reconciliation.md`, `oil-trend-rule.md`). Check there before redesigning a skill from scratch.

## Skills reference

| Skill | Script args | External API |
|-------|-------------|--------------|
| `news` | `--topics`, `--account-topics`, `manage list\|add\|remove` | Google News RSS |
| `cct` | `--mode <pre-market\|eod\|...>` | CCT internal |
| `cct2` | `--mode pre-market\|eod` | Yahoo Finance + dual LLM |
| `stock` | `--market tw\|hk\|all`, `--symbol CODE` | TWSE, Yahoo Finance |
| `chipcon` | `--mode record`, `--deliver-to` | Yahoo Finance chart (SMH/QQQ/SOXX); observation-only report |
| `weather` | `--location NAME` (repeatable) | CWA (Taiwan), HKO (HK) |
| `traffic` | `--from`, `--to`, `--via` | TomTom Routing API |
| `commute` | wraps traffic | TomTom |
| `doughcon` | `--mode deliver\|record`, `--et-hour H` (DST gate) | PizzINT API |
| `oilcon` | `--mode deliver\|record` | Yahoo Finance, Turso |
| `agent-reach` | agent-only, see SKILL.md | 13+ platforms |
| `mindfulness-spirit` | `write`, `fix-signature DEVTO_ID`, `--dry-run` | Google News RSS, dev.to, Turso (via `persona-core` CLI) |
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

### Shared libraries (`lib/`)

| Module | Purpose |
|--------|---------|
| `skill_runner` | Shared agent-first skill runtime helpers (subprocess wrappers, persona-core CLI shortcut, nullclaw agent invocation, cron skill-contract markers). `strip_agent_artifacts()` sanitizes nested-agent stdout before delivery — strips `<ncchoices>` blocks (paired **and** unclosed) plus harness marker lines, so agent protocol noise never reaches Telegram. Any skill that delivers raw `nullclaw agent` stdout must run it through this. |
| `cover_image` | CogView-4 image generation + dev.to cover update CLI |
| `telegram` | Telegram message delivery (auto-detects nullclaw/openclaw config) |
| `delivery` | **Canonical** delivery helper: `deliver_or_fail(chat_id, body, ...)`. Replaces ad-hoc `if args.deliver_to: telegram.send(...)`. On send failure, echoes body to stdout (so cron capture keeps the data) and `exit(1)`; on empty `chat_id`, prints body to stdout. New skills should use this, not `telegram.send` directly. |
| `trace_marker` | Scheduler verification markers: `emit_skill_status()`, `emit_trace()`, `emit_fallback()`. Used by skills with `--verify skill_contract`/`content_has_trace` cron jobs. No-op when `NULLCLAW_JOB_ID` is unset (so manual runs stay clean). Call only *after* delivery confirmation. |
| `heartbeat` | Wall-clock heartbeat for long-running subprocesses |
| `oil_fetch` | Yahoo Finance chart fetch/parse helpers (oilcon) |
| `oil_store` | Turso/libsql time-series storage for `oil_daily` (oilcon) |

`persona_registry` and `persona_history` were retired alongside
`persona-skill`. All persona/history access goes through `persona-core`
CLI now; no Python lib import needed.
