# News LLM 去重 — 實作完成摘要（供 Claude review）

- 日期：2026-07-13
- 狀態：P0 + P1 + P2 全路徑已實作；測試已綠
- 測試：`python3 -m pytest news/scripts/test_run.py -q` → 預期全過

## 變更檔

| 檔案 | 變更 |
|------|------|
| `news/scripts/run.py` | DEDUP_RULES、pair hints、post-select hard dedup、三路徑接入 |
| `news/scripts/test_run.py` | `LlmDedupHintsTests`（fixture + 路徑整合） |
| `news/SKILL.md` | 三層去重文件 |

## 行為

### P0 — `DEDUP_RULES`
共用 prompt 規則：同事件合併 / 同主題不同事件保留 + 免費源文字指引。  
接入：`_summarize_default_section`、`_run_ai_substage`、`_run_custom_topic`。

### P1 — soft pair hints（pre-LLM）
- `_dedup_pair_hints`：`overlap >= 4` 獨立 pair（無 entity、無傳遞合群）
- `_format_dedup_hint_block` 注入 prompt
- `NEWS_LLM_DEDUP_HINTS=0` 關閉
- trace：`llm_dedup_hints`

### P2 — post-select hard dedup（post-LLM）
- `_post_dedup_selected_summary`：marker 驗證後、precheck 前
- 只對已選 `#N`；`overlap >= 4` 連通分量；保留 LLM 順序第一則
- `NEWS_LLM_POST_DEDUP=0` 關閉
- trace：`llm_post_dedup`
- 路徑：default section、AI substage、custom topic

### Cache variants
- AI：`default_ai_clustered_v5_post_dedup`
- custom：`custom_topic_v3_dedup`

### 刻意不做（有害）
- 全量 tech `cluster(overlap=2)`
- `overlap>=3 + entity` 門檻

## 測試覆蓋（`LlmDedupHintsTests`）

- fixture topic-word overlaps（1/5=7, 1/2=3, …）
- pair hints 只出 (1,5)
- independent pairs（無傳遞群組結構）
- format block
- tech prompt：DEDUP_RULES + #1+#5 hint
- AI prompt：DEDUP_RULES、無 tech-style 必需（可無 hint）
- hints kill-switch
- hint trace
- post_dedup 砍 1+5 留 2,4
- post_dedup 不砍 overlap=3 的 1+2
- post_dedup kill-switch
- tech section：post_dedup 在 precheck 前
- AI substage：post_dedup 在 precheck 前
- custom topic：rules + hints + post_dedup

## Review 請寫到

`docs/reviews/news-llm-dedup-claude-review.md`

重點：正確性、插入點、誤併風險、測試缺口、cache/env 契約。可改意見可寫，**不要改業務碼**（除非發現 blocker 級 bug 且必須修——優先寫入 review 檔）。
