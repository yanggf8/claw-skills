# News LLM 去重 — Test plan / inventory

- 日期：2026-07-13
- 測試檔：`news/scripts/test_run.py` → class `LlmDedupHintsTests`
- 執行：`python3 -m pytest news/scripts/test_run.py::LlmDedupHintsTests -q`
- 本檔用途：**測試先寫 / 測試清單**；給 Claude 做 **test-only code review**

## Fixture 真值

`_TECH_DEDUP_FIXTURE_TITLES`（5 則）：

| # | 事件標籤 |
|---|----------|
| 1 | 彭博美半導體危機改寫 A |
| 2 | 記憶體股技術性熊市（不同事件） |
| 3 | 板塊震盪綜述（不同事件） |
| 4 | 輝達選擇權（不同事件） |
| 5 | 彭博美半導體危機改寫 B（與 #1 同事件） |

Token overlap 基線（`_topic_words`）：`1∩5=7`，`1∩2=3`，`1∩3=2`，`2∩4=2`。  
門檻 **4**：僅 `(1,5)` 應觸發 hint / post-dedup 邊。

## 測試清單

| # | 測試方法 | 層 | 驗收 |
|---|----------|----|------|
| T01 | `test_fixture_topic_word_overlaps` | 基線 | overlap 數值固定；門檻回歸時紅燈 |
| T02 | `test_dedup_pair_hints_fixture_only_one_five` | P1 | 只出 `(1,5,7)`；不含 (1,2)(1,3)(2,4) |
| T03 | `test_dedup_pair_hints_independent_not_transitive` | P1 | pair 為獨立 3-tuple；`a<b`；排序確定 |
| T04 | `test_format_dedup_hint_block` | P1 | 空→`""`；有 pair→含 `#1+#5` 與「僅供複核」 |
| T05 | `test_tech_prompt_includes_dedup_rules_and_fixture_hint` | P0+P1 | tech prompt 含 `DEDUP_RULES`、`#1+#5`，不含 `#1+#2` |
| T06 | `test_general_section_gets_hints_and_dedup_rules` | P0+P1 | general 同樣注入 rules + hints |
| T07 | `test_ai_substage_prompt_includes_dedup_rules_without_tech_hints` | P0 | AI 有 `DEDUP_RULES`；無 fixture 時無「可能同事件候選」 |
| T08 | `test_kill_switch_disables_hints_and_traces` | P1 env | `HINTS=0`：無 hint 字串；rules 仍在；trace `enabled=false` |
| T09 | `test_hint_trace_when_enabled` | P1 trace | pairs 僅 `[{a:1,b:5,overlap:7}]` |
| T10 | `test_post_dedup_collapses_one_and_five_keeps_theme_peers` | P2 | 選 1,2,4,5 → 留 1,2,4；砍 5 |
| T11 | `test_post_dedup_does_not_drop_overlap_three_theme_pair` | P2 | 選 1,2（ov=3）兩者皆留 |
| T12 | `test_post_dedup_keeps_first_llm_order_not_lower_id` | P2 | 輸出 `#5` 先於 `#1` → 留 5 砍 1 |
| T13 | `test_post_dedup_noop_single_or_empty` | P2 | 單則/空 summary 不炸 |
| T14 | `test_post_dedup_preserves_non_bullet_lines` | P2 | 前言/結語保留 |
| T15 | `test_post_dedup_kill_switch` | P2 env | `POST_DEDUP=0`：1+5 都留；trace `enabled=false` |
| T16 | `test_tech_section_post_dedup_runs_before_precheck` | 整合 | tech：precheck 只見 survivors |
| T17 | `test_tech_language_fail_branch_still_post_dedups_before_precheck` | 整合 | language fail 分支同樣先 post_dedup |
| T18 | `test_ai_substage_post_dedup_before_precheck` | 整合 | AI：precheck 前已砍同事件 |
| T19 | `test_custom_topic_has_dedup_rules_hints_and_post_dedup` | 整合 | custom：rules+hint+post_dedup |
| T20 | `test_cache_variants_bumped_for_dedup` | cache | AI variant 字串；custom 源碼含 `v3_dedup` |

## 刻意不測（非缺口）

- 真實 nullclaw LLM 輸出（非決定性；一律 mock agent）
- 全量 tech `cluster(overlap=2)`（產品刻意不做）
- entity lexicon 門檻（產品刻意不做）

## Claude review 交付

寫入：`docs/reviews/news-llm-dedup-test-claude-review.md`

審核重點：覆蓋是否完整、假陽性/假陰性、ordering 斷言是否鎖死契約、缺測、脆弱 mock。
