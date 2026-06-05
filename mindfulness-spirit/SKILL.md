---
name: mindfulness-spirit
description: 身心靈 × AI 系列文章自動產出，透過 persona-core 產生、檢查並發布專欄。
version: 0.4.0
author: yanggf
always: false
requires_bins: ["python3", "persona-core", "nullclaw"]
requires_env: ["PERSONA_REGISTRY_DB_URL", "PERSONA_REGISTRY_DB_TOKEN"]
---

# mindfulness-spirit

「身心靈 × AI」系列文章 skill。這裡只保留素材選擇與 prompt 模板；persona、
history、editorial plan、LLM draft/checklist、citation restore、key-link
persistence、dev.to / Telegram delivery、failure dump、plan mark-published 全部由
`persona-core` CLI 負責。

## Script

`~/a/claw-skills/mindfulness-spirit/scripts/run.py`

## Commands

```bash
python3 scripts/run.py write [--dry-run]
python3 scripts/run.py [--dry-run]
python3 scripts/run.py fix-signature DEVTO_ID [--dry-run]
```

`--account` / `--deliver-to` 已移除；發布目的地由 persona-core column
`delivery_target` 決定，避免兩套 routing source of truth。

## Configuration

`~/.nullclaw/config.json` must include:

```json
{
  "skills": {
    "mindfulness_spirit": {
      "persona_slug": "ping-w",
      "publish": true,
      "main_image_url": "https://example.com/cover.png"
    }
  }
}
```

`persona_slug` is required. `publish` and `main_image_url` are still read for
operator visibility, but publish behavior is controlled by the persona-core
column row.

## Flow

1. Fetch Google News RSS with the eight skill-local mindfulness / AI queries.
2. Read stable prompt blocks from persona-core:
   `personas show`, `history list`, and `plans next` with `--as-prompt-block`.
3. Render `prompts/writer.md.tmpl`; pass `prompts/checklist.md.tmpl` through.
4. Write prompt + material TSV files to a temp directory.
5. Run writer → checklist with `nullclaw agent --isolated` inside the skill.
6. `columns installments prepare mindfulness-spirit --print-id`.
7. `columns installments update-body <id>` stores the body, restores
   `[來源 #N]`, and records validation status.
8. `columns installments publish <id>` handles title/signature/secrets,
   delivery, failure dump/alert, history, and plan mark-published.

`--dry-run` stops after RSS fetch, prompt-block reads, and temp-file rendering.
It prints the writer prompt path and does not prepare, draft, or publish.

## Operator Notes

`update-body` persists with `validation_ok=true`. A `validation_summary`
containing `degraded` means the checklist phase failed and the skill
intentionally used the writer output as the deliverable body; treat that as a
partial-success state worth reviewing.

Prompt text lives in `prompts/writer.md.tmpl` and `prompts/checklist.md.tmpl`.
Do not reintroduce `claw-skills/lib/persona_*.py` or `heartbeat.py` imports;
`ainews` still owns those Python helpers until its absorption step.

## Schedule

Weekly Friday 07:00 Asia/Taipei cron entry for the `inner-algorithm` series:

- expression `0 7 * * 5`, timezone `+08:00`
- job id `skill-a1c95bb5-d369-4c86-a6d0-8a03a2e92b19`
- timeout 1800s, `verify=skill_contract`, `repair=retry_once`

The matching seed row is in `~/.nullclaw/cron-seed.json`; do not add a second
concurrent cadence without retiring this one.
