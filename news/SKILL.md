---
name: news
description: Fetch and summarize Taiwan news from Google RSS feeds
always: true
---

# news

Fetch and summarize Taiwan news from Google RSS feeds in Traditional Chinese.

## Script

```
~/.nullclaw/skills/news/scripts/run.py
```

## Usage

```
python3 ~/.nullclaw/skills/news/scripts/run.py --deliver-to 7972814626
```

## Options

- `--lang LANG` — Language (default: zh)
- `--deliver-to CHAT_ID` — Send output directly to Telegram chat instead of printing to stdout
- `--account ACCOUNT` — Telegram bot account name (default: main)
- `--account-topics` — Read topics from `~/.nullclaw/news-topics.json` by account (used by cron)
- `--topics TOPICS` — Comma-separated custom topics (overrides account-topics)

## Topic Management

Users can manage their own news subscriptions via conversation. When a user asks about their topics or wants to add/remove topics, run the management subcommand and reply with the result.

Triggers and commands:
- "我訂閱了什麼" / "我的新聞主題" / "看看我訂閱的新聞" → `python3 ~/.nullclaw/skills/news/scripts/run.py manage list --account ACCOUNT --deliver-to CHAT_ID`
- "加新聞主題 X" / "新增主題 X" / "訂閱 X" → `python3 ~/.nullclaw/skills/news/scripts/run.py manage add --account ACCOUNT --topic X --deliver-to CHAT_ID`
- "移除主題 X" / "取消訂閱 X" / "刪除主題 X" / "不要 X 新聞" → `python3 ~/.nullclaw/skills/news/scripts/run.py manage remove --account ACCOUNT --topic X --deliver-to CHAT_ID`

Replace ACCOUNT with the bot account name and CHAT_ID with the user's Telegram chat ID.

## Prompt

Fetch these four RSS feeds (the `when:1d` parameter is critical — it filters to last 24 hours only):

1. https://news.google.com/rss/search?q=AI+when:1d&hl=zh-TW&gl=TW&ceid=TW:zh-Hant
2. https://news.google.com/rss/search?q=artificial+intelligence+OpenAI+Anthropic+Claude+Gemini+DeepMind+when:1d&hl=en-US&gl=US&ceid=US:en
3. https://news.google.com/rss/search?q=科技+半導體+晶片+when:1d&hl=zh-TW&gl=TW&ceid=TW:zh-Hant
4. https://news.google.com/rss?hl=zh-TW&gl=TW&ceid=TW:zh-Hant

IMPORTANT rules for summarizing:
- Only include news from the last 24 hours. Ignore evergreen/old articles.
- Feed 1 + Feed 2 are your PRIMARY sources for the AI section. Feed 2 is English — translate headlines to Traditional Chinese.
- Feed 3 is for the tech/semiconductor section.
- Feed 4 is for general/breaking news.
- The AI section should ALWAYS have content — if feeds 1+2 return results, there IS AI news today.
- Do NOT say "今日無相關新聞" unless a feed literally returns zero items.

Summarize in Traditional Chinese with this exact format:

📰 早安新聞摘要

**🤖 AI 人工智慧**
- （列出所有 AI 相關項目，不限數量，合併中英文來源，去重）
- （涵蓋：大模型發布、企業AI應用、AI政策監管、AI安全、AI投資併購）

**💻 科技 & 半導體**
- （列出半導體、晶片、消費電子、遊戲、太空科技等非AI科技新聞，不限數量）

**🌏 重大新聞**（最多3則）
- （最重要的3則非科技新聞）

今日重點一句話：（一句話總結今日最重要的事）

## 韌性

腳本會把較長的 LLM 工作切成小階段，避免主機在中途中斷後必須整段重跑。每個完成的子階段會寫入磁碟快取；下一次執行會從已完成的範圍接續。

分段策略：

- **預設新聞模式（`summarize_llm`）**：AI 區塊先做確定性的事件群集去重，再切成兩半（Level 2）。每半是一個 LLM 呼叫（約 14-30 秒）。若某半失敗（逾時、非 0 exit、空 stdout），只把失敗的半段再切成兩個 quarter（Level 3）。若 quarter 仍失敗，整次執行會告警並中止（不再遞迴）。科技與一般新聞維持單次 LLM 呼叫。
- **自訂主題模式（`summarize_llm_custom`，由 `--account-topics` 使用）**：每個主題各跑一次 LLM，依序執行。單一主題失敗時，該主題退回原始標題列表並告警；其他主題繼續送出。

AI 預設新聞去重：

AI 區塊在切半前先做確定性群集。標題會移除 Google News 的尾端 ` - Source`，再抽出 Latin token 與 CJK 字元 bigram；兩則新聞會和既有群集的第一則候選新聞比較，token 重疊數達 `_CLUSTER_OVERLAP = 2` 才歸入同一事件群集。群集的 seed token 不會隨後續新聞擴張，避免不同事件靠累積詞彙串接在一起。群集按來源數排序，代表越多來源報導越重要。每日摘要每個群集只保留 1 則，直接採用該群集中的第一則候選新聞，不再依來源名稱做分類或分支。

CJK bigram 通用詞過濾：常見的中文填充詞（`公司`、`發布`、`布新`、`新產`、`產品`、`股價`、`上漲`、`下跌`）會在 `_CJK_STOP_BIGRAMS` 中被排除，避免「甲公司發布新產品」與「乙公司發布新產品」這類完全無關的標題被視為同一事件。清單由觀察累積，若 trace 中發現新的過度群集模式，請更新 `scripts/run.py` 中的 `_CJK_STOP_BIGRAMS`。

這個去重在 LLM 分段前完成，所以同一事件不會分散到兩個 half。執行時會寫入 `cluster_dedup` trace，欄位包含 `before`、`after`、`clusters_total`、`clusters_kept`。

快取：`~/.nullclaw/.news-cache/<YYYY-MM-DD>/<variant>-<range>.txt`。AI 預設新聞分段使用 `default_ai_clustered_v2` variant，避免重用舊版未群集分段快取。鍵為 `(date, variant, range)`。腳本啟動時會清除 7 天以前的子目錄；需要時可手動刪除。

送出長度處理：

- Telegram 長度以 Markdown 可見文字估算，不用原始 URL byte 數。這可避免 Google News RSS 長 URL 讓實際可讀摘要被誤判過長。
- 若原始 Markdown 太長，腳本會依行切成多段 Telegram 訊息。`digest_delivery_split` trace 會記錄段數與原始/可見字元數。

## Failure alerts (hard rule)

Whenever the skill cannot send the full intended news — this includes any
silent quality degradation — it alerts the operator. Two channels, both
attempted on every failure:

1. Append to `~/.nullclaw/news-failures.log` (plain text, append-only,
   rotated at 1 MiB). This is the durable record and survives Telegram
   outages.
2. Best-effort Telegram message to the same chat the news would have gone
   to (`fail_on_delivery_error=False`, never raises).

Coverage matrix (every path that ends without the full intended news):

| Failure | Behavior |
|---|---|
| All RSS feeds returned 0 items | Alert + exit 1, no digest sent. |
| AI Level 3 quarter still fails | Alert + exit 1, no digest sent. |
| Tech / general fell back to non-LLM bullets | Alert; digest still ships with degraded section. |
| Custom-topic fell back to raw listing | Alert (`custom_topics_fell_back` or `all_custom_topics_failed`); digest still ships. |
| Telegram send failed | Alert via the on-disk log (Telegram itself is down); exit 1. |
| Uncaught exception in main() | Alert with truncated traceback; exit 1. |

檢查單次執行狀態：

```bash
cat ~/.nullclaw/news-failures.log
nullclaw cron trace <job_id_prefix> --event news_failure
nullclaw cron trace <job_id_prefix> --event cluster_dedup
nullclaw cron trace <job_id_prefix> --event ai_substage
```

## Notes

- Delivery: Telegram `7972814626`
- `## Script` runs as `job_type=skill` in cron (no LLM needed for RSS headlines)
- `## Prompt` is used when invoked interactively in Claude Code (LLM summarization)
- Cron verification: use scheduler-owned `skill_contract` with `retry_once`
- After delivery confirmation, cron runs emit `[skill-status:ok|failed]` and `[trace:<NULLCLAW_JOB_ID>]` on separate stdout lines
