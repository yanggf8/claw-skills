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

Summarize in Traditional Chinese. Output is validated to ensure ≥80% CJK characters per bullet and reject common English adverbs (e.g., "increasingly", "significantly"). Only proper nouns may remain in English.

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
- **逾時自動重試（一次）**：任何 LLM 呼叫若是 *逾時*（rc=124，供應商在 stream 開始後卡住、無輸出）會自動重試一次，再退回降級。只重試逾時——驗證失敗、空 stdout、其他非 0 exit 屬確定性失敗，不重試。重試使用較短的 `LLM_RETRY_TIMEOUT_SECS`（預設 30 秒，可用 `NEWS_LLM_RETRY_TIMEOUT` 覆寫），且當 cron 剩餘 wall-clock（`NULLCLAW_SKILL_TIMEOUT` / `NULLCLAW_SKILL_STARTED`）不足以容納重試時直接跳過，避免拖過 cron kill window。trace：`llm_agent_retry`、`llm_agent_retry_skipped_budget`。

AI 預設新聞去重：

AI 區塊在切半前先做確定性群集。標題會移除 Google News 的尾端 ` - Source`，再抽出 Latin token 與 CJK 字元 bigram；兩則新聞會和既有群集的第一則候選新聞比較，token 重疊數達 `_CLUSTER_OVERLAP = 2` 才歸入同一事件群集。群集的 seed token 不會隨後續新聞擴張，避免不同事件靠累積詞彙串接在一起。群集按來源數排序，代表越多來源報導越重要。每日摘要每個群集只保留 1 則，直接採用該群集中的第一則候選新聞，不再依來源名稱做分類或分支。

CJK bigram 通用詞過濾：常見的中文填充詞（`公司`、`發布`、`布新`、`新產`、`產品`、`股價`、`上漲`、`下跌`）會在 `_CJK_STOP_BIGRAMS` 中被排除，避免「甲公司發布新產品」與「乙公司發布新產品」這類完全無關的標題被視為同一事件。清單由觀察累積，若 trace 中發現新的過度群集模式，請更新 `scripts/run.py` 中的 `_CJK_STOP_BIGRAMS`。

這個去重在 LLM 分段前完成，所以同一事件不會分散到兩個 half。執行時會寫入 `cluster_dedup` trace，欄位包含 `before`、`after`、`clusters_total`、`clusters_kept`。

LLM 事件去重（完整三層）：

1. **P0 共用 `DEDUP_RULES`**：default section（tech/general）、AI substage、custom-topic 的 LLM prompt 共用「同事件合併 / 同主題不同事件保留」規則；免費源優先僅為 prompt 文字指引（`pick_representatives` / post-dedup keep 皆採 LLM 順序第一則 — Codex S4 永久 skip 程式端免費源 ranking）。
2. **P1 soft pair hints**（pre-LLM）：對 numbered 候選用 `_topic_words` 算**獨立 pair**，**`overlap >= 4`** 才注入 `可能同事件候選：#A+#B`。hint 列表不做傳遞合群。LLM 仍最終選擇。`NEWS_LLM_DEDUP_HINTS=0` 關閉。trace：`llm_dedup_hints`。套用：default tech/general、**AI substage**、custom-topic。
3. **P2 post-select hard dedup**（post-LLM 安全網）：marker 驗證通過後、precheck 前，**只對 LLM 已選的 `#N`** 做 hard 合併。門檻 **`overlap >= 4`**。演算法是 **greedy 保序**（依 LLM 輸出順序；與已保留則 overlap≥4 則丟），**不做** union-find / 連通分量。`NEWS_LLM_POST_DEDUP=0` 關閉。trace：`llm_post_dedup`。若砍完後則數低於 `pick` 下限且仍非空：記 `post_dedup_underfill`，再做一次 **deterministic refill**（從未被 LLM 選過的 numbered 候選補滿；與每個已保留項 overlap 皆 `<4` 才可補；**不**復活 P2 砍掉的 id、**不**二次 LLM）。trace：`post_dedup_refill`。套用：default tech/general、AI substage、custom-topic。
4. **語言閘**：default / AI / custom 在 post-dedup（含 refill）與 precheck 之後、送出前，皆跑 `_language_validation_passed`；不合格則 `_translate_selected_section`（避免 refill 塞入未翻譯英文 RSS 標題）。

Env（去重相關）：

| 變數 | 預設 | 作用 |
|------|------|------|
| `NEWS_LLM_DEDUP_HINTS` | on（`!=0`） | `=0` 關閉 P1 soft pair hints |
| `NEWS_LLM_POST_DEDUP` | on（`!=0`） | `=0` 關閉 P2 hard dedup + underfill refill |

**Codex 裁決為永久 skip**（非單方面省略；見 `docs/reviews/news-llm-dedup-codex-skips-verdict.md`）：

- **S1** 全量 tech/general 套 `_CLUSTER_OVERLAP=2` pre-cluster：fixture `1∩3=2` 會誤併同主題不同事件；overlap≥4 的 pre-cluster 又與 P1/P2 重複且會在 LLM 前不可逆縮候選 → 維持 P1 hints + P2 greedy。
- **S2** `overlap≥3 + entity lexicon`：fixture `1∩2=3`（美光/三星）會誤連不同事件；entity 詞表無法分辨同實體不同事件 → 門檻維持 ≥4、無 lexicon。
- **S4** 程式端免費源 ranking：屬編輯政策非事件正確性；硬編碼 source 名單會隨付費牆變化失效 → 只保留 prompt 文字偏好。
- **S6** 無第四條未接路徑：default / AI / custom 已對齊 P0+P1+P2（含 custom 路徑 post-dedup 後語言閘，與 AI/default 一致）。

快取：`~/.nullclaw/.news-cache/<YYYY-MM-DD>/<variant>-<range>.txt`。AI 預設新聞分段使用 `AI_SUBSTAGE_CACHE_VARIANT`（目前為 `default_ai_clustered_v5_post_dedup`）variant，避免重用舊版分段快取——prompt 或 cached-output 語意變更時 bump variant 即可讓當日舊快取失效。鍵為 `(date, variant, range)`。腳本啟動時會清除 7 天以前的子目錄；需要時可手動刪除。

送出長度處理：

- Telegram 長度以 Markdown 可見文字估算，不用原始 URL byte 數。這可避免 Google News RSS 長 URL 讓實際可讀摘要被誤判過長。
- 若原始 Markdown 太長，腳本會依行切成多段 Telegram 訊息。`digest_delivery_split` trace 會記錄段數與原始/可見字元數。
- Markdown 字元中和：標題中的 `*` 與 `_` 在送至 Telegram 前會換成全形 `＊` / `＿`，避免 Markdown 解析失敗（例如「長科*成關鍵受惠股」這類台股除權息標示）。僅針對新聞條目本文，不影響區段標題與連結語法。
- Markdown 預檢 + 純文字降級：送出前會檢查每個分塊是否能通過 Telegram Markdown 解析（檢查未配對的 `*` / `_` / `` ` ``、跨塊未閉合的連結括號等）。若任一分塊不安全，整份摘要會降級為純文字送出（`parse_mode=None`），確保整體交付的原子性：要嘛全部以 Markdown 送出，要嘛全部純文字，避免「分塊 1 送出成功，分塊 2 失敗導致 cron 重試又重送分塊 1」。`digest_markdown_unsafe_fallback` trace 會記錄發生降級時的不安全分塊編號。

## Failure alerts (hard rule)

Whenever the skill cannot send the full intended news — this includes any
silent quality degradation — it alerts the operator. Two channels, both
attempted on every failure:

1. Append to `~/.nullclaw/news-failures.log` (plain text, append-only,
   rotated at 1 MiB). This is the durable record and survives Telegram
   outages.
2. Best-effort Telegram message to the same chat the news would have gone
   to (`fail_on_delivery_error=False`, never raises).

每則告警的 `detail` 會附上同一 `(reason, account)` 在過去 30 天的累計次數
（`_recent_failure_count`，例如「此告警近30天已出現 5 次」），讓慢性問題
（例如某模型反覆 thinking-stall 逾時）在告警本身就能被看見，而不是看起來像
一次性事件。資料來源就是上面的 `news-failures.log`，不需額外的 metrics 系統。

Coverage matrix (every path that ends without the full intended news):

| Failure | Behavior |
|---|---|
| All RSS feeds returned 0 items | Alert + exit 1, no digest sent. |
| AI Level 3 quarter still fails | Alert + exit 1, no digest sent. |
| Tech / general fell back to non-LLM bullets | Alert; digest still ships with degraded section. |
| Custom-topic fell back to raw listing | Alert (`custom_topics_fell_back` or `all_custom_topics_failed`); digest still ships. |
| 任一分塊 Markdown 不安全 | 整份降級為純文字送出，正常交付（`digest_markdown_unsafe_fallback` trace）。 |
| Telegram send failed | Alert via the on-disk log (Telegram itself is down); exit 1. |
| Uncaught exception in main() | Alert with truncated traceback; exit 1. |

檢查單次執行狀態：

```bash
cat ~/.nullclaw/news-failures.log
nullclaw cron trace <job_id_prefix> --event news_failure
nullclaw cron trace <job_id_prefix> --event cluster_dedup
nullclaw cron trace <job_id_prefix> --event ai_substage
```

## Content prechecking

The skill sees only RSS titles, so a strong title on paywalled/thin/promo content used
to slip through. Two-tier prechecking (`lib/news_quality.py`) handles it with a **three-way
verdict** per item — `keep`, `drop`, or `title_only`:

- **Tier 1 (every run, no network):** at the dedup site, drops deny-listed **sources** only.
  Promo/listicle judgement is left to the LLM + Tier 2 (the body-tuned patterns over-match
  short legitimate titles like `限時降息`, so they must not hard-gate titles).
- **Tier 2 (only the LLM-picked items, before link attach):** decodes each picked Google
  News link to its real publisher URL (headless `batchexecute`, no browser), then:
  - **drop** — deny source/domain (e.g. `chinatimes.com`) or marketing/PR/listicle body.
  - **title_only** — paywalled/truncated reputable source (e.g. Nikkei/FT). The item is
    **kept**; the LLM's already-Chinese bullet stays as-is (we don't drop it, and we don't
    overwrite it with the raw RSS headline — that would inject English/Japanese past the
    Traditional-Chinese language gate). Counted separately in the trace.
  - **keep** — everything else (LLM bullet retained).
  Runs inside each section function while the `#N` identity still exists, so links stay correct.

**Deterministic** — a paywalled source is always `title_only` whether the body fetch
succeeds or fails (no network-timing nondeterminism). Decode/fetch failure for an unknown
source is `keep` (fail-open). An all-paywalled batch renders headlines, never an empty
section or a false failure.

**Source policy lives in config, not code** — the module ships EMPTY deny/paywall seeds
(no hardcoded publishers). Policy is read from `~/.nullclaw/news-quality-sources.json`
`{"trusted":[...],"deny":[...],"deny_domains":[...],"paywall_domains":[...]}`:
`deny`/`deny_domains` → drop; `paywall_domains` → title_only (keep LLM bullet). Host matching
is suffix-aware (`ft.com` matches `www.ft.com`, not `craft.com`). The repo's working policy
(e.g. `chinatimes.com` deny; Nikkei/FT paywall) lives in that file, so it is operator-owned
and removable without editing source. Because config is union-only (it cannot *remove* a code
seed), keeping seeds empty is what makes any entry removable.

**Starter `paywall_domains`** (add to the config file per host — NOT seeded in code):
```json
{"paywall_domains": ["nytimes.com","cn.nytimes.com","wsj.com","cn.wsj.com","ft.com",
  "ftchinese.com","bloomberg.com","economist.com","nikkei.com","asia.nikkei.com",
  "barrons.com","washingtonpost.com","newyorker.com","wired.com"]}
```

**Paywall free-replacement (double bullet)** — when a `title_only` (paywalled) item
survives selection (i.e. it was the only coverage of its story), the skill searches Google
News RSS and Bing News RSS (keyless) by the headline keywords for a FREE same-story article
from a *different* host, dedupes exact-title hits across sources, translates the winner's
title to Traditional Chinese, and renders a **double bullet**: the free replacement on top,
then a continuation line carrying the original paywalled headline + a
`⚠️ 付費牆（原文需訂閱）` note. If no free replacement is found (or the lookup times out /
errors), it degrades to a single paywalled bullet + the same note — never a hard failure.
A footer `ℹ️ 本次含 N 則付費牆新聞（原文需訂閱）` counts paywalled *stories* (once each). The
lookup is bounded and defensive (any exception → no replacement), preserving the exit-0
contract. Trace: `paywall_replacement_found`, `paywall_replace_deadline`, `paywall_notice`.
Env knobs: `NEWS_PAYWALL_REPLACE=0` disables the lookup (note-only);
`NEWS_PAYWALL_REPLACE_DEADLINE` (default 20s), `NEWS_PAYWALL_REPLACE_MAX` (default 4) bound it;
`NEWS_PAYWALL_REPLACE_SOURCES` (default `google,bing`) selects which RSS indexes to query;
`NEWS_PAYWALL_REPLACE_BING_MKT` (default `en-US`) sets Bing's market parameter.

Env knobs: `NEWS_PRECHECK=0` disables both tiers; `NEWS_PRECHECK_DECODE_TIMEOUT`,
`NEWS_PRECHECK_FETCH_TIMEOUT`, `NEWS_PRECHECK_DEADLINE`, `NEWS_PRECHECK_WORKERS` bound latency
(the deadline force-cancels stragglers so wall-clock is capped). Trace events
`quality_tier1` / `quality_tier2` record counts, drops, and title_only conversions.

## Notes

- Delivery: Telegram `7972814626`
- `## Script` runs as `job_type=skill` in cron (no LLM needed for RSS headlines)
- `## Prompt` is used when invoked interactively in Claude Code (LLM summarization)
- Cron verification: use scheduler-owned `skill_contract` with `retry_once`
- After delivery confirmation, cron runs emit `[skill-status:ok|failed]` and `[trace:<NULLCLAW_JOB_ID>]` on separate stdout lines
