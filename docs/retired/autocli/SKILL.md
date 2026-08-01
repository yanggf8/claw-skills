---
name: autocli
description: 透過 AutoCLI 從 55+ 網站取得結構化資料（HackerNews、Reddit、Bilibili、Twitter/X、YouTube 等）
always: false
---

# AutoCLI

透過 [AutoCLI](https://github.com/nashsu/AutoCLI) 從 55+ 網站取得結構化資料。支援 HackerNews、Reddit、Bilibili、Twitter/X、YouTube、知乎、小紅書等 333 個命令。

## 前置條件

安裝 AutoCLI（一次性）：

```bash
curl -fsSL https://raw.githubusercontent.com/nashsu/autocli/main/scripts/install.sh | sh
```

需要瀏覽器登入的網站（如 Twitter/X、Bilibili 動態）另需安裝 Chrome 擴充套件，參見 [AutoCLI README](https://github.com/nashsu/AutoCLI#chrome-extension-setup)。

## Script

`scripts/run.py`

## 用法

```bash
# 取得 HackerNews 熱門
python3 scripts/run.py hackernews top --limit 10

# 取得 Bilibili 熱門
python3 scripts/run.py bilibili hot --limit 15

# 列出所有可用網站與命令
python3 scripts/run.py list
# 列出特定網站的子命令
autocli hackernews --help

# 原始 JSON 輸出（供 agent 使用）
python3 scripts/run.py hackernews top --limit 5 --raw

# 發送到 Telegram
python3 scripts/run.py hackernews top --limit 10 --deliver-to <chat_id> --account main
```

## 選項

| 選項 | 說明 |
|---|---|
| `site` | 網站名稱（如 hackernews、bilibili、reddit）或 `list` |
| `command` | 命令名稱（如 top、hot、search） |
| `--limit N` | 最大取得筆數 |
| `--timeout N` | 逾時秒數（預設 90） |
| `--raw` | 輸出原始 JSON |
| `--deliver-to ID` | Telegram chat ID |
| `--account NAME` | Telegram bot 帳號（預設 main） |

其餘未知參數會直接轉交 autocli（如搜尋關鍵字）。

## 輸出

人類可讀模式輸出編號列表，自動截斷至 3800 字元以符合 Telegram 限制。`--raw` 模式輸出 autocli 原始 JSON。

## Cron 排程範例

```bash
# 即時測試（一次性，1 分鐘後執行）
nullclaw cron once 1m autocli --skill-args "hackernews top --limit 5" \
  --deliver-to <chat_id>

# 每天早上 8 點取得 HackerNews 熱門
nullclaw cron add-skill "0 8 * * *" autocli \
  --skill-args "hackernews top --limit 10" \
  --deliver-to <chat_id> --account main --timeout 120 \
  --verify content_has_trace --repair retry_once

# 每 6 小時取得 Bilibili 熱門
nullclaw cron add-skill "0 */6 * * *" autocli \
  --skill-args "bilibili hot --limit 15" \
  --deliver-to <chat_id> --timeout 120 --verify exit_only

# 每天取得 Reddit 熱門
nullclaw cron add-skill "0 9 * * *" autocli \
  --skill-args "reddit top --limit 10" \
  --deliver-to <chat_id> --verify exit_only
```

## 注意事項

- 部分網站（如 Twitter/X、Bilibili 動態）需要瀏覽器登入，錯誤訊息會提示安裝 Chrome 擴充套件
- autocli 管理自己的設定與認證，本技能不需要 config.json
- 失敗時不會發出 trace marker，確保 `content_has_trace` 驗證能正確反映執行狀態
