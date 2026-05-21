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

## Resilience

The script splits long LLM work into small substages so a host crash mid-run
does not have to start over. Each completed substage is cached on disk; the
next attempt picks up where the last one died.

Substaging policy:

- **Default-feed mode (`summarize_llm`)**: the AI section's items are split into
  two halves (Level 2). Each half is one LLM call (~14-30s). On per-half
  failure (timeout / non-zero exit / empty stdout), only that half is split
  one more time into quarters (Level 3). If a quarter still fails, the run
  is aborted with an alert (no recursion past Level 3). Tech and general
  remain single calls.
- **Custom-topics mode (`summarize_llm_custom`, used by `--account-topics`)**:
  one LLM call per topic, sequential. On per-topic failure that topic falls
  back to a raw bullet listing of recent titles (still useful), and an alert
  fires for the degraded topic. Other topics continue normally.

Cross-source dedup (default-feed AI section, 2026-05-19):

Each per-section / per-substage prompt asks the LLM to prefer free-source
coverage (cnyes, TechNews, Yahoo新聞, MoneyDJ, 工商時報, Reuters, AP,
ScienceDaily, TechCrunch) over paid outlets (WSJ, Bloomberg, FT, Nikkei,
Barron's) when multiple sources cover the same story (same company-quarter
financials, same policy announcement, same product launch, same research
breakthrough). Paid-only stories are kept. After the two AI halves are
merged, `_crosshalf_dedup` runs one more LLM pass over the joined bullets
to catch stories that landed in both halves with different sources. Fails
open: any timeout/parse error returns the unfiltered input — a redundant
digest beats a missing one. Trace events: `ai_substage_crosshalf_dedup`,
`crosshalf_dedup_{exception,nonzero_exit,empty_stdout,no_ids_parsed,kept_nothing}`.

Cache: `~/.nullclaw/.news-cache/<YYYY-MM-DD>/<variant>-<range>.txt`. Keyed by
`(date, variant, range)`. Swept on script start: subdirectories older than 7
days are deleted. Safe to wipe manually.

Delivery length handling:

- Telegram message length is checked by visible Markdown text, not raw URL
  bytes. This prevents long Google News RSS URLs from causing links to be
  stripped while the user-visible digest still fits.
- When raw Markdown is too large for one safe Telegram POST, the script splits
  the digest on line boundaries and sends numbered chunks. The trace event
  `digest_delivery_split` records the chunk count plus raw/visible character
  counts.

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

To inspect what happened on a given run, use:

```bash
cat ~/.nullclaw/news-failures.log
nullclaw cron trace <job_id_prefix> --event news_failure
nullclaw cron trace <job_id_prefix> --event ai_substage
```

## Notes

- Delivery: Telegram `7972814626`
- `## Script` runs as `job_type=skill` in cron (no LLM needed for RSS headlines)
- `## Prompt` is used when invoked interactively in Claude Code (LLM summarization)
- Cron verification: use scheduler-owned `skill_contract` with `retry_once`
- After delivery confirmation, cron runs emit `[skill-status:ok|failed]` and `[trace:<NULLCLAW_JOB_ID>]` on separate stdout lines
