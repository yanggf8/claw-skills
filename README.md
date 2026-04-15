# claw-skills

Personal agent skills. **Dual-agent**: one source tree runs against both `nullclaw` and `openclaw` hosts.

## Current agent support

| Agent      | Status    | Config file                       | Telegram schema                                         | Install location                     |
|------------|-----------|-----------------------------------|---------------------------------------------------------|--------------------------------------|
| nullclaw   | supported | `~/.nullclaw/config.json`         | `channels.telegram.accounts.<name>.bot_token` (multi)   | symlink each dir to `~/.nullclaw/skills/` |
| openclaw   | supported | `~/.openclaw/openclaw.json`       | `channels.telegram.botToken` (single)                   | the whole repo lives at `<workspace>/skills/` |

Same Python scripts, same `SKILL.md` frontmatter, same `lib/telegram.py`. The `CLAW_CONFIG` / `CLAW_ENV` env vars pick which host's config to read; the schema is then auto-detected (nullclaw's multi-account `accounts.<name>.bot_token` tried first, openclaw's single `botToken` fallback). The `--account` flag is a no-op on openclaw.

**What is not shared**: the agent CLI itself (`nullclaw ...` vs `openclaw ...`), cron scheduling commands, and `SKILL.md`'s `## Script` line (which hardcodes nullclaw paths — openclaw's loader ignores it and locates `scripts/run.py` relative to `SKILL.md`).

## Install

**nullclaw** (default): symlink or copy into the nullclaw skills dir.

```bash
ln -s ~/a/claw-skills/<skill> ~/.nullclaw/skills/<skill>
# or: cp -r ~/a/claw-skills/<skill> ~/.nullclaw/skills/<skill>
```

**openclaw**: this whole repo lives inside your OpenClaw workspace. Per OpenClaw's skill loader, each skill must resolve (via realpath) to a path inside `<workspace>/skills/`. The simplest layout is to keep this git repo directly at `<workspace>/skills/`:

```bash
# One-time setup
mv ~/a/claw-skills ~/clawd/skills         # the whole repo becomes the skills dir
export CLAW_CONFIG="$HOME/.openclaw/openclaw.json"   # add to ~/.profile
```

Verify:

```bash
cd ~/clawd && openclaw skills list        # skills show source=openclaw-workspace
```

The lib/, README.md, CLAUDE.md, and .git at the repo root are harmless — OpenClaw only loads immediate subdirs that contain a SKILL.md.

## Config resolution

`lib/telegram.py` and the `load_env()` helpers read from the first path that exists:

1. `$CLAW_CONFIG` / `$CLAW_ENV` if set
2. `~/.nullclaw/config.json` / `~/.nullclaw/.env` (default)

Config schema auto-detected:

- **nullclaw**: `channels.telegram.accounts.<name>.bot_token` (multi-account)
- **openclaw**: `channels.telegram.botToken` (single token; `--account` is ignored)

## Skills

| Skill | Description |
|-------|-------------|
| agent-reach | Check if the agent (nullclaw or openclaw) is reachable |
| cct | Fetch CCT 4-moment trading intelligence and deliver to Telegram |
| cct2 | Dual-LLM market sentiment analysis for configured tickers |
| commute | Fetch route travel time via traffic sub-skill |
| doughcon | Fetch PizzINT DOUGHCON level and deliver or record |
| lib | Shared Python helpers (telegram delivery) — not a skill itself |
| news | Fetch and summarize Taiwan news from Google RSS feeds |
| oilcon | Fetch oil futures levels and deliver or record a daily snapshot |
| stock | Fetch TWSE/HSI market indices and individual stock quotes |
| traffic | Fetch TomTom route travel time between waypoints |
| weather | Fetch weather forecast for Taiwan (CWA) and Hong Kong (HKO) |

## Gotchas

- **`weather` name collision (openclaw)**: OpenClaw ships a bundled `weather` skill (wttr.in). Workspace skills take precedence, so this repo's `weather` wins. Rename the folder + frontmatter `name:` if you want both.
- **Taiwan weather needs `CWA_API_KEY`**: put it in `~/.nullclaw/.env` (default) or `~/.openclaw/.env` and export `CLAW_ENV=$HOME/.openclaw/.env`.
- **OpenClaw symlink rule**: the loader `realpath`s every candidate and rejects anything outside `<workspace>/skills/`. Symlinking skills in from a sibling dir like `~/clawd/external-skills/` does **not** work.
