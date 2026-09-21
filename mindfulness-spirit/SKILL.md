---
name: mindfulness-spirit
description: 身心靈 × AI 系列文章自動產出，透過 persona-core 產生、檢查並發布專欄。
version: 0.4.0
author: yanggf
always: false
requires_bins: ["persona-core", "nullclaw"]
requires_env: ["PERSONA_REGISTRY_DB_URL", "PERSONA_REGISTRY_DB_TOKEN"]
---

# mindfulness-spirit

「身心靈 × AI」系列文章 skill。這裡只保留素材選擇與 prompt 模板；persona、
history、editorial plan、LLM draft/checklist、citation restore、key-link
persistence、dev.to / Telegram delivery、failure dump、plan mark-published 全部由
`persona-core` CLI 負責。

## Script

```
~/.nullclaw/skills/mindfulness-spirit/bin/mindfulness-spirit
```

Rust (`crates/mindfulness-spirit`). The Python was removed on 2026-08-02;
recover it from git history if a rollback is ever needed. Publish the binary
with `tools/install-skill.sh mindfulness-spirit`, never by hand.

## Commands

```bash
~/.nullclaw/skills/mindfulness-spirit/bin/mindfulness-spirit write [--dry-run]
~/.nullclaw/skills/mindfulness-spirit/bin/mindfulness-spirit [--dry-run]
~/.nullclaw/skills/mindfulness-spirit/bin/mindfulness-spirit fix-signature DEVTO_ID [--dry-run]
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
      "column_slug": "machine-and-cushion",
      "publish": true,
      "main_image_url": "https://example.com/cover.png"
    }
  }
}
```

`persona_slug` is required. `column_slug` selects the running season and
defaults to the current one; starting the next season is a config edit, not a
code change. `publish` and `main_image_url` are still read for operator
visibility, but publish behavior is controlled by the persona-core column row.

## Flow

0. Emit `[skill-status:ok]` / `[trace:]` on success. The Python emitted
   neither, so every run — including the ones that published — was recorded as
   `content_invalid` by the `skill_contract` cron.
1. Fetch Google News RSS with the eight skill-local mindfulness / AI queries
   (five English, three Chinese; the locale follows the query).
2. Read stable prompt blocks from persona-core:
   `personas show`, `history list`, and `plans next` with `--as-prompt-block`.
3. Render `prompts/writer.md.tmpl`; pass `prompts/checklist.md.tmpl` through.
4. Write prompt + material TSV files to a temp directory.
5. Run writer → checklist with `nullclaw agent --isolated` inside the skill.
6. `columns installments prepare <column_slug> --print-id`.
7. `columns installments update-body <id>` stores the body, restores
   `[來源 #N]`, and records validation status.
8. `columns installments publish <id>` handles title/signature/secrets,
   delivery, failure dump/alert, history, and plan mark-published.

`--dry-run` stops after RSS fetch, prompt-block reads, and temp-file rendering.
It prints the writer prompt path and does not prepare, draft, or publish.

## Operator Notes

`update-body` persists with `validation_ok=true`, and it is only ever reached
after the checklist passed. **A failed checklist aborts** — nothing is
prepared, stored or published, the installment stays `planned`, and next
week's run picks up the same one. An earlier version published the unreviewed
writer output under a `degraded` summary; that made the label the only
difference between reviewed and not, and the skill's own tests were written to
forbid it. This section described that behaviour for months after it was
removed.

An exhausted season is exit 4 from `columns installments prepare` —
persona-core's not-found. The binary prints a hint naming both fixes. That
state produced six consecutive weeks of Friday failures once, diagnosed as a
broken skill.

Prompt text lives in `prompts/writer.md.tmpl` and `prompts/checklist.md.tmpl`,
resolved relative to the binary at `<skill>/bin/<name>`. A slot the renderer
cannot fill is an error, not a blank: a silently-empty `{topic_block}` yields
an article that reads fine and is no longer part of a series.

## Schedule

Weekly Friday 07:00 Asia/Taipei cron entry. The season it writes for is
`skills.mindfulness_spirit.column_slug`, currently `machine-and-cushion`:

- expression `0 7 * * 5`, timezone `+08:00`
- job id `skill-a1c95bb5-d369-4c86-a6d0-8a03a2e92b19`
- timeout 1800s, `verify=skill_contract`, `repair=retry_once`

The matching seed row is in `~/.nullclaw/cron-seed.json`; do not add a second
concurrent cadence without retiring this one.
