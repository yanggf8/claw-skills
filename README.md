# claw-skills

Personal agent skills. One source tree runs against **nullclaw**, **openclaw**, and **nanoclaw** hosts.

## Current agent support

| Agent      | Status    | Config file                       | Telegram schema                                         | Install location                     |
|------------|-----------|-----------------------------------|---------------------------------------------------------|--------------------------------------|
| nullclaw   | supported | `~/.nullclaw/config.json`         | `channels.telegram.accounts.<name>.bot_token` (multi)   | symlink each dir to `~/.nullclaw/skills/` |
| openclaw   | supported | `~/.openclaw/openclaw.json`       | `channels.telegram.botToken` (single)                   | the whole repo lives at `<workspace>/skills/` |
| nanoclaw   | supported | `~/.nullclaw/config.json` (default) or `$CLAW_CONFIG` | same as nullclaw or openclaw (auto-detected) | symlink each dir to `<nanoclaw>/container/skills/<skill>` |

Same Rust binaries, same `SKILL.md` frontmatter, same `claw-core` delivery. The `CLAW_CONFIG` / `CLAW_ENV` env vars pick which host's config to read; the schema is then auto-detected (nullclaw's multi-account `accounts.<name>.bot_token` tried first, openclaw's single `botToken` fallback). The `--account` flag is a no-op on openclaw.

The `SKILL.md` format is the standard Claude Code skill format — all three agents use the same frontmatter (`name`, `description`, `always`). The `## Script` line hardcodes nullclaw paths as documentation; nanoclaw's container agent locates `bin/<name>` relative to `SKILL.md`, the same way openclaw does.

**What is not shared**: the agent CLI itself and the cron scheduling commands. The binaries are self-contained, so a nanoclaw container needs no interpreter — build them for the container's target and mount them in.

## Install

**nullclaw** (default): symlink or copy into the nullclaw skills dir.

```bash
ln -s ~/claw/claw-skills/<skill> ~/.nullclaw/skills/<skill>
# or: cp -r ~/claw/claw-skills/<skill> ~/.nullclaw/skills/<skill>
```

**openclaw**: this whole repo lives inside your OpenClaw workspace. Per OpenClaw's skill loader, each skill must resolve (via realpath) to a path inside `<workspace>/skills/`. The simplest layout is to keep this git repo directly at `<workspace>/skills/`:

```bash
# One-time setup
mv ~/claw/claw-skills ~/clawd/skills      # the whole repo becomes the skills dir
export CLAW_CONFIG="$HOME/.openclaw/openclaw.json"   # add to ~/.profile
```

Verify:

```bash
cd ~/clawd && openclaw skills list        # skills show source=openclaw-workspace
```

README.md, CLAUDE.md, crates/ and .git at the repo root are harmless — OpenClaw only loads immediate subdirs that contain a SKILL.md.

**nanoclaw**: symlink each skill dir into nanoclaw's container skills directory.

```bash
ln -s ~/claw/claw-skills/<skill> ~/claw/nanoclaw/container/skills/<skill>
```

The container agent discovers `bin/<name>` relative to `SKILL.md` — no path changes needed. Build the binaries for the container's architecture and make sure `CLAW_CONFIG` / `CLAW_ENV` point at your config paths.

## Config resolution

`claw-core`'s telegram and env helpers read from the first path that exists:

1. `$CLAW_CONFIG` / `$CLAW_ENV` if set
2. `~/.nullclaw/config.json` / `~/.nullclaw/.env` (default)

Config schema auto-detected:

- **nullclaw**: `channels.telegram.accounts.<name>.bot_token` (multi-account)
- **openclaw**: `channels.telegram.botToken` (single token; `--account` is ignored)

## Skills

| Skill | Description |
|-------|-------------|
| agent-reach | Give the agent internet eyes — search/read 17 platforms (Twitter/X, Reddit, YouTube, GitHub, Bilibili, etc.) and any web page |
| cct | Fetch CCT 4-moment trading intelligence and deliver to Telegram |
| cct2 | Dual-LLM market sentiment analysis for configured tickers — pre-market and EOD reports |
| cds-con | Report corporate credit-spread levels and how many stored observations sit below each — signal-only, deliberately unclassified (no status ladder) |
| chipcon | Observe SMH semiconductor momentum via trend/relative-strength signals; signal-only observation report |
| doughcon | Fetch PizzINT DOUGHCON level and deliver or record |
| inflation-con | Judge whether inflation is genuinely persistent from FRED core-PCE / core-CPI / breakeven data — signal-only evidence packet |
| liko-finance-weekly | Draft, validate, and publish the weekly liko-finance cross-border wealth signal stream |
| mindfulness-spirit | 身心靈 × AI series — auto-draft, check, and publish columns via persona-core |
| news | Fetch and summarize Taiwan news from Google RSS feeds |
| oilcon | Fetch oil futures levels and deliver or record a daily regime snapshot |
| stock | Fetch TWSE/HSI market indices and individual stock quotes |
| traffic | Fetch TomTom route travel time between waypoints |
| weather | Fetch weather forecast for Taiwan (CWA) and Hong Kong (HKO) |

Descriptions come from each skill's `SKILL.md` frontmatter — **when you add a skill, add its row here and in `CLAUDE.md`'s skills table.** Both tables missed `cds-con` and `inflation-con` for weeks.

`crates/` is the Cargo workspace, not a skill — OpenClaw's loader ignores it since it has no `SKILL.md`.

## Gotchas

- **`weather` name collision (openclaw)**: OpenClaw ships a bundled `weather` skill (wttr.in). Workspace skills take precedence, so this repo's `weather` wins. Rename the folder + frontmatter `name:` if you want both.
- **Taiwan weather needs `CWA_API_KEY`**: put it in `~/.nullclaw/.env` (default) or `~/.openclaw/.env` and export `CLAW_ENV=$HOME/.openclaw/.env`.
- **OpenClaw symlink rule**: the loader `realpath`s every candidate and rejects anything outside `<workspace>/skills/`. Symlinking skills in from a sibling dir like `~/clawd/external-skills/` does **not** work.
