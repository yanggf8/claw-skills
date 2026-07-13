# Grok corroboration of Codex review

- 日期：2026-07-13
- 輸入：`docs/reviews/news-llm-dedup-codex-review.md`
- 對照：`news/scripts/run.py`、`news/scripts/test_run.py`、提案檔
- 方法：逐條抽查 Codex 行號與關鍵斷言；不改業務碼

## 總結

| 項目 | 結果 |
|------|------|
| Codex verdict | `approve-with-changes` — **採納** |
| 行號引用 | 抽查函式起點與關鍵區段，**吻合** |
| 核心結論 P0 keep/change、P1 收窄、P2 首批 drop | **同意** |
| 需修正的提案點 | P1 entity 門檻自打臉 #1/#2 — **同意 Codex** |

## 逐項核對

| Codex 斷言 | 核對 | 結果 |
|------------|------|------|
| `dedup` 440–449 exact title | 函式在 440；`title.lower()` | PASS |
| `cluster` / `pick` 僅 AI path | `pick_representatives` 呼叫僅 1768（定義 495）；tech 走 `_summarize_default_section` | PASS |
| `pick` 取群內第一則、無免費源排序 | 499 `group[:per_cluster]`；測試 `test_pick_representatives_uses_cluster_order_without_source_labels` 在 375 | PASS |
| 免費源優先只在 prompt | `cnyes` 僅 1534、1684 兩處 prompt 字串 | PASS |
| tech 有「事件本身」規則、AI substage 無 | 1537 有；1683–1686 有多來源無「事件本身」 | PASS |
| tech vs AI 後處理順序不同 | tech：marker→language?→precheck→paywall→translate/links；AI：marker→precheck→paywall→language→links | PASS |
| precheck 依 marker IDs | `_precheck_apply` 893 起 | PASS |
| #1/#5=7、#1/#2=3 等 overlap | 本機 `_topic_words` 重算與提案/Codex 一致 | PASS |
| P1 `overlap>=3+entity` 會誤提示 #1+#2 | 共同 `美光/三星/晶片`；需求又要分開 | PASS — 提案自相矛盾 |
| 首批 drop P2 | hard-delete + 同門檻誤傷 #1/#2；且 free-source 在 precheck 前不可靠 | PASS（意見合理） |
| MVP = P0 + tech-only P1(overlap≥4 pair, no entity, no transitive) | 對本 bug #1/#5(7) 可提示、#1/#2(3) 不觸 | PASS |

## 對 Codex 意見的採納

**採納的 ship set：**

1. 共用 `DEDUP_RULES`（P0）給 tech + AI substage  
2. AI prompt 變更 → bump `AI_SUBSTAGE_CACHE_VARIANT`  
3. tech-only soft pair hints：`overlap >= 4`，獨立 pair list，不做 entity / union-find  
4. `NEWS_LLM_DEDUP_HINTS=0` + `llm_dedup_hints` trace  
5. 5-title fixture 測試  
6. **首批不做 P2 hard dedup**

**未採納／保留疑慮：**

- Codex 未讀 `lib/news_quality`（其自標 unverified）— 同意暫不依賴  
- `overlap >= 4` 仍非語義真值；canary 必要 — 同意  

## 流程紀錄（關於 shell vs plugin）

| 階段 | 做法 | 結果 |
|------|------|------|
| 早期 | shell 直接 `codex-companion.mjs` + poll | 違約；job 易殭屍/被殺 |
| 修正後 | `codex:codex-rescue` plugin + 檔案交換 | 產出 `news-llm-dedup-codex-review.md` |
| 本檔 | Grok 對原始碼 corroborate | 本檔 |

交接檔齊全：

- `docs/reviews/news-llm-dedup-proposal.md`
- `docs/reviews/news-llm-dedup-codex-review.md`
- `docs/reviews/news-llm-dedup-corroboration.md`
- `docs/reviews/news-llm-dedup-work.log`
