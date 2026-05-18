---
name: liko-finance-weekly
description: Draft, validate, and publish the weekly liko-finance cross-border wealth signal stream
always: false
---

# liko-finance-weekly

Runs the weekly `weekly-intl-wealth-signals` stream for the
`liko-finance` persona.

The skill delegates durable state and delivery to `persona-core`; it does
not call raw SQL, raw dev.to `curl`, or Telegram APIs directly.

## Script

```
~/.nullclaw/skills/liko-finance-weekly/scripts/run.py
```

## Usage

Dry run:

```bash
python3 ~/.nullclaw/skills/liko-finance-weekly/scripts/run.py --dry-run
```

Live weekly run:

```bash
python3 ~/.nullclaw/skills/liko-finance-weekly/scripts/run.py
```

## Workflow

1. `persona-core streams issues prepare weekly-intl-wealth-signals`
2. `persona-core personas get liko-finance`
3. `persona-core streams get weekly-intl-wealth-signals`
4. Read the stream-specific source policy:
   `docs/superpowers/specs/2026-04-29-liko-finance-weekly-design/sources.md`
5. Ask the nullclaw agent to draft exactly one issue body in liko's voice.
6. `persona-core streams issues validate-body @<draft>`
7. If valid, write back with `streams issues update-body`.
8. Publish with `streams issues publish <id> --target both`.

## Safety

- `--dry-run` drafts and validates only; it does not write the issue body
  and does not publish.
- The publish step is entirely handled by `persona-core`, which loads the
  dev.to API key and Telegram bot token internally.
- The skill emits `[skill-status:ok|failed]` and `[trace:<job_id>]` for
  nullclaw `skill_contract` verification.

## Cron

Sunday 09:00 Asia/Taipei:

```bash
nullclaw cron add-skill "0 9 * * 0" liko-finance-weekly \
  --timeout 1800 --tz +08:00 --verify skill_contract --repair retry_once
```
