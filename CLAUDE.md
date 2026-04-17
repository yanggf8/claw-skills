# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this repo is

Personal agent skills — Python scripts invoked as cron jobs or on-demand by the **nullclaw**, **openclaw**, or **nanoclaw** agent. Each skill lives in its own directory. Same source, same `SKILL.md` format, three hosts.

The `cct` skill (CCT trading intelligence) lives separately in `~/a/cct/skills/cct/` and is versioned with that project.

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

The `lib/` directory is a shared Python package, not a skill. All scripts import it via:
```python
SKILLS_LIB = os.path.join(os.path.dirname(__file__), "..", "..", "lib")
sys.path.insert(0, os.path.abspath(SKILLS_LIB))
import telegram
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
2. Script must: accept `--deliver-to` and `--account`, import `telegram` from lib, exit 0 on API errors
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

## Skills reference

| Skill | Script args | External API |
|-------|-------------|--------------|
| `news` | `--topics`, `--account-topics`, `manage list\|add\|remove` | Google News RSS |
| `stock` | `--market tw\|hk\|all`, `--symbol CODE` | TWSE, Yahoo Finance |
| `weather` | `--location NAME` (repeatable) | CWA (Taiwan), HKO (HK) |
| `traffic` | `--from`, `--to`, `--via` | TomTom Routing API |
| `commute` | wraps traffic | TomTom |
| `doughcon` | `--mode deliver\|record` | PizzINT API |
| `oilcon` | `--mode deliver\|record` | Yahoo Finance, Turso |
| `agent-reach` | agent-only, see SKILL.md | 13+ platforms |
| `mindfulness-spirit` | `write`, `fix-signature DEVTO_ID`, `--dry-run` | Google News RSS, dev.to, Turso |
| `persona-skill` | `get\|list\|create\|update\|delete`, `set-secret\|get-secret\|delete-secret`, `history`, `plan-list\|plan-show` | Turso |

### Shared libraries (`lib/`)

| Module | Purpose |
|--------|---------|
| `persona_registry` | Persona CRUD, secrets, schema v1 |
| `persona_history` | Publish history, editorial plans, schema v2 |
| `cover_image` | CogView-4 image generation + dev.to cover update CLI |
| `telegram` | Telegram message delivery (auto-detects nullclaw/openclaw config) |
| `heartbeat` | Wall-clock heartbeat for long-running subprocesses |
