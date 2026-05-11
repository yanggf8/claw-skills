---
name: mindfulness-spirit
description: 身心靈 × AI 系列文章自動產出，支援編輯計畫驅動的系列寫作，發布到 dev.to。
version: 0.3.0
author: yanggf
always: false
requires_bins: ["python3", "nullclaw", "persona-core"]
requires_env: ["PERSONA_REGISTRY_DB_URL", "PERSONA_REGISTRY_DB_TOKEN"]
---

# mindfulness-spirit

「身心靈 × AI」主題的系列文章自動產出引擎。由 Turso 中的編輯計畫驅動，依序產出系列文章，發布到 dev.to。作者身份、寫作風格、API 金鑰全部從 Turso persona-registry 載入。

## Status: migrated to persona-core (2026-05-11)

This skill is now fully integrated into [persona-core](https://github.com/yanggf8/persona-core).
The migration is complete — there is **no separate "mindfulness-spirit" data
layer to maintain**. Everything below uses persona-core's Turso schema:

- **Column metadata**: `persona-core`'s `content_column` table holds the
  active column row (`slug='inner-algorithm'`, `theme='mindfulness'`,
  `skill='mindfulness-spirit'`, `status='active'`, `persona_slug='ping-w'`).
- **Per-issue history**: `persona-core`'s `persona_history` table records
  every published article (`skill='mindfulness-spirit'`,
  `stream='inner-algorithm'`, with dev.to URL and timestamp).
- **Persona profile + secrets**: `persona-core`'s `persona_registry` table
  holds the writer persona (Ping W.) and the dev.to API key.
- **Editorial plan**: `persona-core`'s `editorial_plan` table holds the
  series topics; `next_topic()` and `mark_topic_published()` SQL
  contracts are locked by tests in `cli-rs/src/commands/history.rs`.

**For users / future agents**: when you want to inspect or modify this
skill's behavior beyond the prompts in this SKILL.md, use the persona-core
CLI directly. Examples:

```bash
persona-core personas get ping-w                    # writer profile
persona-core history list --skill mindfulness-spirit --limit 10
persona-core streams issues prepare inner-algorithm  # next topic + draft scaffold
```

Do **not** introduce a parallel persona-store, persona-CLI, or
mindfulness-spirit-only Turso schema. Anything you'd want for column
writing belongs in persona-core.

**Implementation note (transitional)**: the live `scripts/run.py` still
imports `persona_history` and `persona_registry` from
`~/a/claw-skills/lib/` rather than shelling out to the `persona-core` CLI.
Both lib functions write to the same Turso DB persona-core uses, so
behavior is identical. The lib-import will be cut to a `persona-core`
shell-out in a follow-up. Until then, the lib remains the only consumer
of these Python modules — they are **not** intended as a public API.

## Subcommands

```bash
# 預設寫作流程（write 可省略）
python3 scripts/run.py write [--dry-run] [--deliver-to CHAT_ID] [--account NAME]
python3 scripts/run.py [--dry-run] [--deliver-to CHAT_ID] [--account NAME]

# 修補已發布文章的作者署名
python3 scripts/run.py fix-signature DEVTO_ID [--dry-run]
```

## Configuration

於 `~/.nullclaw/config.json`：

```json
{
  "skills": {
    "mindfulness_spirit": {
      "publish": true,
      "main_image_url": "https://raw.githubusercontent.com/yanggf8/claw-skills/master/mindfulness-spirit/assets/inner-algorithm-cover.png",
      "persona_slug": "ping-w"
    }
  }
}
```

- `publish` (bool, default `true`): 預設直接發布；設為 `false` 會建立草稿。
- `main_image_url` (string, optional): dev.to 文章 cover image。系列共用同一張。
- `persona_slug` (string): persona-registry 中的 slug。從 Turso 載入 role、name、expression、mental_models 等全部欄位。
- 環境變數 `MINDFULNESS_SPIRIT_MAIN_IMAGE_URL` 會覆蓋 config 中的 `main_image_url`。
- 環境變數 `MINDFULNESS_SPIRIT_PERSONA_ROLE` 會覆蓋所有其他 persona 設定（緊急用）。

## Persona & secrets

Persona identity is loaded from the `persona-registry` Turso DB via `lib/persona_registry`. The full persona (including expression, mental_models, heuristics, antipatterns, limits) drives the writer's voice.

dev.to API key resolution order (first non-empty wins):

1. `persona_secret` row — `persona-core secrets set <slug> devto_api_key <key>` (preferred; per-persona)
2. `DEV_TO_API_KEY` environment variable (legacy, logs a back-compat warning)

## Editorial plan integration

When an `editorial_plan` exists for this skill in Turso, the write flow automatically:

1. Queries `next_topic()` for the next `planned` topic in the active series
2. Injects the topic's `angle`, `lens`, `direction`, and `key_question` into the writer prompt
3. After successful publish, calls `mark_topic_published()` to link the topic to its history row

The active series slug is `inner-algorithm` (hardcoded in `SERIES_SLUG`). Use `persona-core streams issues list inner-algorithm` to view topic status.

## Anti-repetition

The writer prompt includes:
- Recent publish history (titles, stances, key_links) from `persona_history`
- 4 explicit anti-repetition rules forbidding reuse of titles, opening patterns, metaphors, and angles

## Author signature

Every article ends with an auto-derived signature:

```
*—— <span translate="no">Ping W.</span>（在宗教界服務的心行者）*
```

The `translate="no"` span prevents translation services from mangling the author name. The signature is injected by the writer prompt and preserved by the checklist prompt.

## Cover image

Series-level cover image stored at `assets/inner-algorithm-cover.png` and set via `main_image_url` config. Generate new images with `lib/cover_image.py`:

```bash
python3 lib/cover_image.py generate "prompt" -o assets/cover.png
python3 lib/cover_image.py update-devto ARTICLE_ID IMAGE_URL --persona ping-w
```

## 執行流程

1. **Editorial plan**: 查詢 Turso 中的下一個 `planned` 主題，注入 angle/lens/direction/key_question。
2. **RSS 抓取**: 抓取 Google News 中關於身心靈與 AI 的最新消息。
3. **作家 LLM**: 結合主題指引和素材，寫出 2000-3500 字的繁體中文初稿。自動存到 `runs/<run_id>-writer.md`。
4. **檢查清單 LLM**: 用寫作檢查清單自評，優化標題、開頭、節奏與金句。
5. **連結還原**: 只把 `[來源 #N]` 還原為原始連結。
6. **dev.to 發布**: 帶入 cover image，發布到 dev.to。失敗時 markdown 落地到 `failed/`。
7. **History 記錄**: 寫入 `persona_history` 表，標記 editorial topic 為 `published`。
8. **Telegram 通知**: 成功時發送標題、摘要與文章連結；失敗時發送階段與原因。

長時間 LLM 呼叫每 30 秒在 stderr 輸出 `[phase] still running (Ns elapsed)` 進度訊息。
