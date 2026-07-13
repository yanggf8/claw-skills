# Claude full code review request — news LLM dedup

- 日期：2026-07-13
- 範圍：**全部**本功能寫過的碼（實作 + 測試 + SKILL）
- 測試：`python3 -m pytest news/scripts/test_run.py -q` 應全綠

## 必讀檔

1. `news/scripts/run.py` — 焦點：
   - `DEDUP_RULES`
   - `_dedup_pair_hints` / `_format_dedup_hint_block`
   - `_parse_pick_min`
   - `_post_dedup_selected_summary`（greedy P2）
   - `_refill_unselected_after_underfill`（Codex S3）
   - `_summarize_default_section`（hints + post_dedup + pick_min）
   - `_run_ai_substage`（hints Codex S5 + post_dedup）
   - `_run_custom_topic`（hints + post_dedup + cache variant）
   - `AI_SUBSTAGE_CACHE_VARIANT` / env flags
2. `news/scripts/test_run.py` — `LlmDedupHintsTests` 全 class + fixture
3. `news/SKILL.md` — LLM 事件去重段落（含 Codex-approved skip 措辭）

## 背景裁決（只作意圖，以碼為準）

- `docs/reviews/news-llm-dedup-codex-skips-verdict.md`（S1–S7）
- `docs/reviews/news-llm-dedup-fix-log.md`

## 請寫出

`docs/reviews/news-llm-dedup-claude-full-review.md`

結構：

1. Verdict: approve | approve-with-changes | request-changes
2. Scope reviewed
3. Findings by severity (blocker / major / minor / nit) with file:function + evidence
4. Implementation correctness (P0/P1/P2/refill, ordering, cache, env)
5. Test quality (gaps, false confidence, brittle mocks)
6. SKILL.md accuracy vs code
7. Recommended fixes (ordered)
8. Ready to merge? yes/no + why

規則：

- **Read-only**：不要改 `news/scripts/` 或 `lib/`
- 可 append `docs/reviews/news-llm-dedup-work.log`：`ISO | CLAUDE | …`
- 證據必須對到實際行號／行為；繁中優先

Done when the review file exists with all 8 sections.
