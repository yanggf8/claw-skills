# News skill：LLM 去重改進方案

- 日期：2026-07-13
- 範圍：`news/` skill（default AI / tech / general 摘要路徑）
- 狀態：提案（待 Codex 審查 + Grok corroborate）
- 交接檔：
  - 本檔：提案
  - `docs/reviews/news-llm-dedup-codex-review.md`：Codex 審查輸出（待寫）
  - `docs/reviews/news-llm-dedup-corroboration.md`：Grok 核對（待寫）
  - `docs/reviews/news-llm-dedup-work.log`：過程 log

---

## 1. 問題現象

科技區塊出現同一事件改寫標題的雙重收錄，例如：

1. 川普「晶片回流」恐夢碎？專家揭美半導體業最大危機：台積電、美光、三星都難逃
2. 美光、三星領跌 記憶體晶片股集體陷技術性熊市
3. 半導體股震盪何時完結？
4. 晶片股慘遭拋售 輝達選擇權卻爆量押多 甦醒時刻到了？
5. 台積電也難逃！彭博爆美國晶片業「拉警報」 最大危機曝光

**真正同事件重複：#1 與 #5**（同為彭博美半導體危機／台積電難逃的改寫稿）。  
#2–#4 屬同主題不同事件（記憶體股 / 板塊綜述 / 輝達選擇權），不應硬併。

---

## 2. 現況（已對原始碼核對）

| 機制 | 位置 | 行為 | 對本 bug |
|------|------|------|----------|
| exact `dedup()` | `news/scripts/run.py` `dedup` | 標題字串完全相同才丟 | 擋不住改寫標題 |
| `cluster` + `pick_representatives` | 僅 `_summarize_default_ai_substaged` | AI 切半前事件群集，每群 1 代表 | **tech 不跑** |
| `_CLUSTER_OVERLAP = 2` | CJK bigram + Latin token | 重疊 ≥2 歸群 | 若硬套 tech：#1+#5 能併，但 #1+#2+#3 會誤併 |
| LLM 去重規則 | `_summarize_default_section` prompt | 要求同事件只留一則、免費源優先 | soft rule；本次漏掉 #1/#5 |
| AI substage prompt | `_run_ai_substage` | 有多來源只挑一則，**沒有** tech 那條「事件本身」規則 | 較弱 |
| 免費源優先 | prompt 文字 only | **程式沒有**依 source 排序代表 | `pick_representatives` 取群內第一則 |

### 測得的 token overlap（`_topic_words`，本機重算）

| pair | overlap | 共用 |
|------|---------|------|
| 1 vs 5 | **7** | 危機、台積、大危、晶片、最大、積電、難逃 |
| 1 vs 2 | 3 | 三星、晶片、美光 |
| 1 vs 3 | 2 | 半導、導體 |
| 2 vs 4 | 2 | 晶片、片股 |

### tech 路徑處理順序（成功路徑）

```
_number_items_for_prompt
  → _run_nullclaw_agent (LLM)
  → _marker_validation_stats
  → _language_validation_passed?
       fail → _precheck_apply → _resolve_paywall_replacements → _translate_selected_section
       pass → _precheck_apply → _resolve_paywall_replacements → _attach_numbered_links
```

---

## 3. 目標 / 非目標

### 目標

- 降低 tech（必要時 AI）「同事件多出口改寫」重複。
- 保留「同主題不同主體」多則共存。
- 不拉爆 cron 時限；可 env kill-switch。

### 非目標

- 不要把 AI 那套 `overlap=2` 全量 cluster 直接套到 tech 全候選。
- 不要為每區加第二輪完整摘要 LLM（除非審查認為必要且可接受）。
- 不重寫 paywall / precheck 架構。

---

## 4. 方案分層

### P0 — Prompt hardening（LLM 本體）

1. 抽出共用常數 `DEDUP_RULES`，供 `_summarize_default_section` 與 `_run_ai_substage` 共用。
2. 明確區分：
   - **同事件（應合併）**：同一電稿／財報／政策／研究；標題 hook 不同仍算同一事件。
   - **同主題不同事件（應保留）**：主體不同（記憶體板塊 vs 輝達選擇權 vs 泛板塊綜述）。
3. 放具體正／反例：#1+#5 必須併；#2 與 #4 必須分。
4. 合併時仍以免費源優先、付費牆為 tie-break（延續現有 prompt 精神）。

### P1 — Soft cluster hints（確定性協助，不硬砍）

1. 對 numbered 候選算 pairwise `_topic_words` overlap。
2. 僅在高訊號時注入提示，例如：
   - overlap ≥ 4，或
   - overlap ≥ 3 **且** 至少一個 entity-like bigram 交集（台積／輝達／美光／三星／OpenAI 等）。
3. prompt 附加：`可能同事件候選組: #1+#5; ...`
4. LLM 仍做最終選擇。
5. trace：`llm_dedup_hints`。

### P2 — Post-LLM selected-set hard dedup（小 N 安全網）

1. LLM 回 `#N` 後，**只對已選 3–5 則**做確定性去重（不是全 feed）。
2. 門檻嚴於全量 cluster（建議 overlap ≥ 3 且 entity 交集非空，或 overlap ≥ 4）。
3. 代表選擇：盡量落實免費源優先（**今天程式沒有，需新增 helper**；否則維持 LLM 順序第一則）。
4. 插入位置建議：**marker 驗證通過後、precheck 之前**（或 precheck 後、paywall replace 前）— 需審查確認，避免 paywall map / double-bullet 身份錯位。
5. trace：`llm_post_dedup`（before/after marker ids）。

### P3 — 測試

1. 5 則 fixture：hint builder 必須提示 (1,5)；不得把 1/2/3/5 合成一組。
2. post-select：collapse 1+5，保留 2 與 4。
3. prompt 字串：tech 與 AI path 都含 `DEDUP_RULES`。

### P4 — 上線

1. tech 先上 P0+P1+P2；AI 至少 P0（已有 pre-cluster；P2 可選）。
2. env：`NEWS_LLM_DEDUP_HINTS=0`、`NEWS_LLM_POST_DEDUP=0`。
3. 若 AI prompt 變更：bump `AI_SUBSTAGE_CACHE_VARIANT`。
4. 更新 `news/SKILL.md` 去重段落。

---

## 5. 請 Codex 回答的開放問題

1. P1 門檻在現有 `_topic_words` / stop bigrams 下是否仍會誤提示無關半導體新聞？
2. P2 是否比把 pre-cluster 擴到 tech 更安全、ROI 更高？
3. AI substage 已做 pre-cluster，還需要 P2 嗎？
4. 與 marker validation / paywall replacement / language validation 的插入點衝突？
5. hint 計算延遲可忽略嗎？entity lexicon 維護成本？
6. 遺漏 failure mode？更小的最小可行方案？

---

## 6. Codex 輸出契約

請把完整審查寫入：

`docs/reviews/news-llm-dedup-codex-review.md`

結構固定：

1. Verdict：`approve` / `approve-with-changes` / `reject`
2. Corroborated facts（path + function + 行號）
3. Incorrect or risky claims
4. Per-package critique P0–P4（keep / change / drop）
5. Recommended minimal ship set（有序）
6. Tests + acceptance criteria（5-title fixture）
7. Do-not-do list
8. Answers to open questions 1–6

規則：

- 可寫上述 review 檔；**不要改業務程式**。
- 可讀 `news/scripts/run.py`、`news/scripts/test_run.py`、`news/SKILL.md`、本提案。
- 邊做可 append 一行到 `docs/reviews/news-llm-dedup-work.log`（ISO 時間 + 短狀態）。
- 證據與意見分開。繁中優先；識別子英文 OK。
- 不要再繞 bailian-cli / bl；純本機讀碼即可。
