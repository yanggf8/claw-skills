# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this repo is

Personal nullclaw skills — Python scripts that run as cron jobs or on-demand via the nullclaw agent. Each skill lives in its own directory and is symlinked into `~/.nullclaw/skills/<name>` on the host machine.

The `cct` skill (CCT trading intelligence) lives separately in `~/a/cct/skills/cct/` and is versioned with that project.

## Skill structure

Every skill directory contains:
- `SKILL.md` — frontmatter (`name`, `description`, `always: true`) + usage docs. This is what nullclaw reads.
- `scripts/run.py` — the executable. Always exits 0 (prints a `[WARN: ...]` on failure instead of raising).

The `lib/` directory is a shared Python package, not a skill. All scripts import it via:
```python
SKILLS_LIB = os.path.join(os.path.dirname(__file__), "..", "..", "lib")
sys.path.insert(0, os.path.abspath(SKILLS_LIB))
import telegram
```

## Running skills

```bash
python3 ~/.nullclaw/skills/<name>/scripts/run.py [options]

# Examples
python3 ~/.nullclaw/skills/stock/scripts/run.py --market tw
python3 ~/.nullclaw/skills/news/scripts/run.py --deliver-to 7972814626
python3 ~/.nullclaw/skills/weather/scripts/run.py --location 臺北市
```

## Installing / registering with nullclaw

```bash
# Symlink (already done on this machine)
ln -s ~/a/claw-skills/<skill> ~/.nullclaw/skills/<skill>

# Verify
nullclaw skills list
```

## Telegram delivery

`lib/telegram.py` provides `send(chat_id, text, account="main")`. Bot token is read from `~/.nullclaw/config.json`:
```
channels.telegram.accounts.<account>.bot_token
```

All scripts accept `--deliver-to CHAT_ID` and `--account NAME`. When `--deliver-to` is omitted, output goes to stdout (useful for cron debugging).

## SKILL.md frontmatter

```yaml
---
name: skill-name
description: One-line description shown in nullclaw skills list
always: true        # load into agent context automatically
---
```

The `## Script` section tells nullclaw what to run for `job_type=skill` cron jobs. The `## Prompt` section (if present) is used when the skill is invoked interactively via the LLM agent instead.

## Adding a new skill

1. Create `<skill>/SKILL.md` and `<skill>/scripts/run.py`
2. Script must: accept `--deliver-to` and `--account`, import `telegram` from lib, exit 0 on API errors
3. `ln -s ~/a/claw-skills/<skill> ~/.nullclaw/skills/<skill>`
4. `nullclaw skills list` to confirm

## Cron scheduling

Skills are scheduled via nullclaw cron, not crontab:
```bash
nullclaw cron add-skill "35 13 * * 1-5" <skill> --deliver-to <chat_id> --skill-args "<args>"
nullclaw cron list
nullclaw cron backup
```

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
