# 仍「未做」項目 — 請 Codex 逐項裁決

- 日期：2026-07-13
- 操作者規則：**禁止單方面「刻意不做」**；要 skip 必須 Codex 明確 `skip` 並給理由。
- 現況：P0/P1/P2 已落地（P2 = greedy 保序、overlap≥4、underfill trace、無二次 LLM）。測試 89 green。
- 請讀：`news/scripts/run.py`、`news/SKILL.md` LLM 去重段、本檔。

## 裁決契約

對每一項輸出：

```
ID: S1
Verdict: implement | skip
Rationale: …
If implement: minimal design (funcs, thresholds, tests, risks)
If skip: why safe permanently (not "MVP later")
```

寫入：`docs/reviews/news-llm-dedup-codex-skips-verdict.md`  
可 append：`docs/reviews/news-llm-dedup-work.log`（`ISO | CODEX | …`）

**不要改程式**（本輪只裁決）。

---

## S1 — 全量 tech/general feed pre-cluster（`_CLUSTER_OVERLAP=2`）

- 現狀：AI 有；tech/general **沒有**（SKILL 寫「刻意不做」）
- 理由曾說：overlap=2 會把同主題不同事件誤併（fixture 1∩3=2）
- 問：是否仍 skip？若 implement，門檻是否必須 ≥4 才套用 cluster？是否只 cluster 不砍、只當排序？

## S2 — `overlap≥3 + entity lexicon` 門檻

- 現狀：統一 `overlap≥4`、**無 entity**
- 理由曾說：≥3+美光/三星 會誤連 #1+#2
- 問：是否永遠 skip entity？有無「可維護的小詞表」值得做？

## S3 — underfill 後二次 LLM refill

- 現狀：`post_dedup_underfill` 只 trace，**不 refill**
- 問：是否 implement 一次 deterministic refill（從未選 numbered 補滿 pick_min，不重跑 LLM）？還是二次 LLM？還是 skip？

## S4 — 程式端免費源 ranking

- 現狀：只在 `DEDUP_RULES` 文字；`pick_representatives` / post-dedup keep 皆 **LLM 順序第一**
- 問：是否在 post-dedup keep 決策中 implement source_name 免費源優先（名單從哪來）？

## S5 — AI substage 注入 P1 soft hints

- 現狀：AI 有 pre-cluster + P0 rules + P2；**無** soft pair hint 注入
- 問：是否也 inject hints（與 tech 同門檻）？

## S6 — custom-topic 以外路徑是否還有遺漏

- 現狀：default section / AI / custom 皆有 P0+P1(hints)+P2（AI 無 P1 hints）
- 問：還有沒有必須對齊的呼叫點？

## S7 — SKILL.md「刻意不做」措辭

- 問：裁決後是否改成「Codex 批准 skip：…」並刪除單方面「刻意不做」用語？

---

## 操作者後續

- `implement` 項 → 立刻實作 + 測試  
- `skip` 項 → 寫入 SKILL 為 **Codex-approved skip**（非 AI 單方面省略）
