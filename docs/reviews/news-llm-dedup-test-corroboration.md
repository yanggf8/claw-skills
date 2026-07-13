# Grok corroboration — Claude test review + follow-up fixes

- 日期：2026-07-13
- 輸入：`docs/reviews/news-llm-dedup-test-claude-review.md`（verdict: approve-with-changes）
- 另對照：`docs/reviews/news-llm-dedup-claude-review.md`（實作 major: P2 union-find 傳遞誤刪）

## Claude 測試審查核對

| Claude 結論 | 核對 | 處理 |
|-------------|------|------|
| T01–T20 大體 covered well | PASS | 同意 |
| M1 T07 AI 無 hint 斷言 vacuously true | PASS | 已改：餵 1/5，斷言無 `#1+#2` / 無「可能同事件候選」 |
| M2 T03 未鎖 pair 集合 | PASS | 已改：`assertEqual(pairs, [(1,2,12),(1,3,12),(2,3,12)])` |
| m1 custom cache 未鎖落地檔名 | PASS | 已在 T19 斷言 `custom_topic_v3_dedup` 出現在 cache 檔名 |
| m2 post_dedup trace 未斷 dropped/pairs | PASS | 已在 T10 斷 before/after/dropped/pairs/min_overlap |
| m3 T18/T19 未斷最終 lines | PASS | 已補 assertIn/assertNotIn |
| m4 門檻常數未鎖 | PASS | 已斷 HINT==POST==4、預設 enabled |
| 實作 M1 P2 union-find 傳遞誤刪 | PASS 可實證 | **P2 改 greedy 保序**，加 `test_post_dedup_no_transitive_bridge_drop` |

## 測試現況

```
python3 -m pytest news/scripts/test_run.py -q
# 83 passed
python3 -m pytest news/scripts/test_run.py::LlmDedupHintsTests -q
# 21+ cases (含 bridge / language-fail / general / custom / thresholds)
```

## 檔案

| 檔 | 角色 |
|----|------|
| `docs/reviews/news-llm-dedup-test-plan.md` | 測試清單（T01–T20+） |
| `docs/reviews/news-llm-dedup-test-claude-review.md` | Claude 測審 |
| `docs/reviews/news-llm-dedup-test-corroboration.md` | 本檔 |
| `docs/reviews/news-llm-dedup-claude-review.md` | Claude 實作審 |
| `news/scripts/test_run.py` | `LlmDedupHintsTests` |
