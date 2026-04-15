---
name: mindfulness-spirit
description: 每天自動產出一篇「身心靈 × AI」主題的長文章，直接發布到 dev.to。
version: 0.1.1
author: nullclaw-agent
always: false
requires_bins: ["python3", "nullclaw"]
requires_env: ["DEV_TO_API_KEY"]
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
      "main_image_url": "https://example.com/header.jpg"
    }
  }
}
```

- `publish` (bool, default `true`): 預設直接發布；設為 `false` 會建立草稿。
- `main_image_url` (string, optional): dev.to 文章 header image。
- 環境變數 `MINDFULNESS_SPIRIT_MAIN_IMAGE_URL` 會覆蓋 config 中的 `main_image_url`。
- 環境變數 `DEV_TO_API_KEY` 會覆蓋 `skills.dev_to_api_key`。

## 選項

- `--dry-run`: 跑流程但不發 dev.to、不發 Telegram
- `--skip-editor`: 只跑作家（除錯用）
- `--deliver-to CHAT_ID`: 指定 Telegram 接收者
- `--account ACCOUNT`: 指定 config 中的 Telegram account

## 執行流程

1. **RSS 抓取**: 抓取 Google News 中關於身心靈與 AI 的最新消息。
2. **作家 LLM**: 吸收素材，寫出 2000-3500 字的繁體中文初稿。
3. **編輯 LLM**: 資深編輯視角，優化標題、開頭、節奏與金句。
4. **連結還原**: 只把 `[來源 #N]` 還原為原始連結，不碰一般數字方括號。
5. **dev.to 發布**: 預設 `published=true` 直接發布到 dev.to；若 `skills.mindfulness_spirit.main_image_url` 有設定（或環境變數 `MINDFULNESS_SPIRIT_MAIN_IMAGE_URL`），會同時帶入文章 header image。可將 `publish` 設為 `false` 改成草稿模式。若 API key 缺失、為 placeholder 或 API 失敗，markdown 會落地到 `failed/`。
6. **Telegram 通知**: 成功時發送標題、摘要與文章連結；失敗時發送階段與原因。
