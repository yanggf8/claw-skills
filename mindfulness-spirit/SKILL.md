---
name: mindfulness-spirit
description: 每天自動產出一篇「身心靈 × AI」主題的長文章，直接發布到 dev.to。
version: 0.2.0
author: nullclaw-agent
always: false
requires_bins: ["python3", "nullclaw"]
requires_env: ["PERSONA_REGISTRY_DB_URL", "PERSONA_REGISTRY_DB_TOKEN"]
---

# mindfulness-spirit

每天自動產出一篇「身心靈 × AI」主題的長文章，直接發布到 dev.to。定位為世界宗教博物館基金會執行室的 AI 助手作品。

## Script

```
scripts/run.py
```

執行時需透過已安裝到 runtime 的副本（例如 `~/.claude/skills/mindfulness-spirit/scripts/run.py`）。本 repo 為 canonical 來源，runtime 必須是獨立副本，不可使用 symlink。

## 觸發方式

```bash
python3 scripts/run.py [選項]
```

## Configuration

於 `~/.nullclaw/config.json`：

```json
{
  "skills": {
    "dev_to_api_key": "...",
    "mindfulness_spirit": {
      "publish": true,
      "main_image_url": "https://example.com/header.jpg",
      "persona_slug": "ping-w",
      "persona": {
        "role": "世界宗教博物館基金會執行室的駐站作家",
        "name": null,
        "voice_notes": null
      }
    }
  }
}
```

- `publish` (bool, default `true`): 預設直接發布；設為 `false` 會建立草稿。
- `main_image_url` (string, optional): dev.to 文章 header image。
- `persona_slug` (string, optional): persona-registry 中的 slug（如 `"ping-w"`）。設定後會從 Turso 載入 role、name、expression 等，優先於 `persona` 物件。未設定或 slug 不存在時 fallback 到 `persona` 物件。
- `persona` (object, optional): 作家身份。`role` 預設為 `世界宗教博物館基金會執行室的駐站作家`；`name` 與 `voice_notes` 目前保留給未來的 persona skill 使用。
- 環境變數 `MINDFULNESS_SPIRIT_MAIN_IMAGE_URL` 會覆蓋 config 中的 `main_image_url`。
- 環境變數 `MINDFULNESS_SPIRIT_PERSONA_ROLE` 會覆蓋所有其他 persona 設定（最高優先）。

## Persona & secrets

Persona identity is loaded from the `persona-registry` Turso DB via `lib/persona_registry`. Set `persona_slug` in config to pick the persona.

dev.to API key resolution order (first non-empty wins):

1. `persona_secret` row — `persona-skill set-secret <slug> devto_api_key` (preferred; per-persona)
2. `DEV_TO_API_KEY` environment variable (legacy, logs a back-compat warning)
3. `skills.dev_to_api_key` in `~/.nullclaw/config.json` (legacy)

Required env vars: `PERSONA_REGISTRY_DB_URL` and `PERSONA_REGISTRY_DB_TOKEN`. If the registry is unreachable, persona resolution falls back to the inline `persona` object in config (lenient policy).

## 選項

- `--dry-run`: 跑流程但不發 dev.to、不發 Telegram
- `--deliver-to CHAT_ID`: 指定 Telegram 接收者
- `--account ACCOUNT`: 指定 config 中的 Telegram account

## 執行流程

1. **RSS 抓取**: 抓取 Google News 中關於身心靈與 AI 的最新消息。
2. **作家 LLM**: 吸收素材，寫出 2000-3500 字的繁體中文初稿。完成後自動存到 `runs/<run_id>-writer.md`。
3. **檢查清單 LLM**: 用寫作檢查清單自評，優化標題、開頭、節奏與金句。
4. **連結還原**: 只把 `[來源 #N]` 還原為原始連結，不碰一般數字方括號。
5. **dev.to 發布**: 預設 `published=true` 直接發布到 dev.to；若 `skills.mindfulness_spirit.main_image_url` 有設定（或環境變數 `MINDFULNESS_SPIRIT_MAIN_IMAGE_URL`），會同時帶入文章 header image。可將 `publish` 設為 `false` 改成草稿模式。若 API key 缺失、為 placeholder 或 API 失敗，markdown 會落地到 `failed/`。
6. **Telegram 通知**: 成功時發送標題、摘要與文章連結；失敗時發送階段與原因。

長時間 LLM 呼叫（作家、檢查清單）每 30 秒在 stderr 輸出 `[phase] still running (Ns elapsed)` 進度訊息，方便除錯時掌握階段狀態。
