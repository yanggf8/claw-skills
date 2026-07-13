# News LLM 去重 — Test-only code review (Claude)

- 審核者：Claude (Opus 4.8)
- 日期：2026-07-13
- 範圍：**test-only**。只審 `news/scripts/test_run.py` → `class LlmDedupHintsTests`（含 fixture `_TECH_DEDUP_FIXTURE_TITLES`）。`run.py` 僅用來核對測試是否綁到真契約。未修改 `news/scripts/` 或 `lib/`。
- 執行結果：`python3 -m pytest news/scripts/test_run.py::LlmDedupHintsTests -q` → **20 passed in 0.08s**（本機實跑）。

---

## 1. Verdict

**approve-with-changes**

測試套件品質高：斷言綁在真實符號上（`run.DEDUP_RULES`、`run._dedup_pair_hints`、`run._post_dedup_selected_summary`、`run.AI_SUBSTAGE_CACHE_VARIANT`），不是字串複製；門檻回歸（T01）、ordering（T16/T17/T18）、first-LLM-order（T12）、kill-switch（T08/T15）都鎖到位。20 條全綠且對應真契約。

之所以不是 straight approve，是有**兩個 major false-confidence 破口**與若干 minor 缺測（見 §3、§4），它們讓「綠燈」在特定實作退化下仍會綠。不阻擋進入實作審查，但建議先補 §7 前三項。

---

## 2. Coverage map（T01–T20）

| # | 方法 | 綁定契約 | 評級 | 備註 |
|---|------|----------|------|------|
| T01 | `test_fixture_topic_word_overlaps` | `run._topic_words` 真值 | **covered well** | 鎖死 1∩5=7 / 1∩2=3 / 1∩3=2 / 2∩4=2。門檻回歸的守門員。 |
| T02 | `test_dedup_pair_hints_fixture_only_one_five` | `_dedup_pair_hints` | **covered well** | 正例 `(1,5,7)` + 三條負例（不誤命中 same-theme）。真陽/真陰都測。 |
| T03 | `test_dedup_pair_hints_independent_not_transitive` | `_dedup_pair_hints` | **weak** | 見 §3-M2：只斷言「每個 pair 是 3-tuple、a<b、排序」，**沒斷言 A-B-C 鏈確實產生哪些獨立 pair**（例如未斷言含 `(1,3)` 或不含某 group）。無法區分「正確發獨立 pair」與「漏發 pair」。 |
| T04 | `test_format_dedup_hint_block` | `_format_dedup_hint_block` | **covered well** | 空→`""`；含 `#1+#5`、`僅供複核`、`endswith("\n\n")`。格式契約鎖死。 |
| T05 | `test_tech_prompt_includes_dedup_rules_and_fixture_hint` | tech section prompt | **covered well** | prompt 含 `DEDUP_RULES`、`#1+#5`，不含 `#1+#2`，含 `可能同事件候選`。P0+P1 同時鎖。 |
| T06 | `test_general_section_gets_hints_and_dedup_rules` | general section prompt | **covered well** | 證明 general 走同一注入路徑（非 tech 專屬）。 |
| T07 | `test_ai_substage_prompt_includes_dedup_rules_without_tech_hints` | AI substage prompt | **weak** | 見 §3-M1：`assertNotIn("可能同事件候選")` **vacuously true**——AI 路徑結構上根本無 hint 注入碼，此斷言測不出任何東西。`DEDUP_RULES` 那半是真綁定。 |
| T08 | `test_kill_switch_disables_hints_and_traces` | `LLM_DEDUP_HINTS_ENABLED` | **covered well** | 關掉後：無 hint 字串、`DEDUP_RULES` 仍在、trace `enabled=False, pairs=[]`。三面都斷。 |
| T09 | `test_hint_trace_when_enabled` | `llm_dedup_hints` trace | **covered well** | 精確 `[{"a":1,"b":5,"overlap":7}]`。key 名與值都鎖（overlap 而非 ov）。 |
| T10 | `test_post_dedup_collapses_one_and_five_keeps_theme_peers` | `_post_dedup_selected_summary` | **covered well** | 選 1,2,4,5 → 留 1,2,4；砍 5；`assertNotIn("#5")`。正砍 + 不誤砍同時測。 |
| T11 | `test_post_dedup_does_not_drop_overlap_three_theme_pair` | 同上 | **covered well** | overlap=3 兩者皆留。門檻下邊界（false-merge 防線）。 |
| T12 | `test_post_dedup_keeps_first_llm_order_not_lower_id` | 同上 union-find keep 規則 | **covered well** | `#5` 先出 → 留 5 砍 1。這是最容易被「取 min id」實作寫錯的契約，測得好。 |
| T13 | `test_post_dedup_noop_single_or_empty` | 同上邊界 | **covered well** | 單則/空字串不炸；空回傳 strip 後為 `""`。 |
| T14 | `test_post_dedup_preserves_non_bullet_lines` | 同上 | **covered well** | 前言/結語保留、bullet 砍。行分類正確性。 |
| T15 | `test_post_dedup_kill_switch` | `LLM_POST_DEDUP_ENABLED` | **covered well** | 關掉：1+5 都留；trace `enabled=False`。 |
| T16 | `test_tech_section_post_dedup_runs_before_precheck` | tech 整合 ordering | **covered well** | 用 `fake_precheck` 捕捉 `precheck_ids==[[1,2]]`，證明 precheck 只見 survivors。真正鎖住「post_dedup 在 precheck 前」。 |
| T17 | `test_tech_language_fail_branch_still_post_dedups_before_precheck` | language-fail 分支 ordering | **covered well** | 覆蓋 §run.py 1746 的 fail 分支，precheck 仍 `[[1,2]]`。這條分支很容易在重構時漏掉 post_dedup，測得好。 |
| T18 | `test_ai_substage_post_dedup_before_precheck` | AI 整合 ordering + renumber | **covered well** | items `[1,5,2]` → renumber `#1,#2,#3`，#2 併入 #1 → `precheck_ids==[[1,3]]`。真實走 `_number_items_for_prompt` 重新編號，不是假資料。 |
| T19 | `test_custom_topic_has_dedup_rules_hints_and_post_dedup` | custom 整合 | **covered well** | rules + `#1+#5` hint + `precheck_ids==[[1,2]]`。三件事一次鎖。 |
| T20 | `test_cache_variants_bumped_for_dedup` | cache variant | **covered well（但見 §6）** | AI variant 字面 `default_ai_clustered_v5_post_dedup`；custom 用 `inspect.getsource` 抓 `custom_topic_v3_dedup`。防 silent revert 的設計正確。 |

**統計**：covered well ×17、weak ×3（T03、T07 部分、T20 見 §6 註）、missing ×0。

---

## 3. Findings by severity

### Major

**M1 — T07 的「無 tech hints」斷言是假陽性守門（`test_ai_substage_prompt_includes_dedup_rules_without_tech_hints`）**
`run._run_ai_substage`（run.py 1832–1929）**結構上沒有任何 hint 注入碼**——不呼叫 `_dedup_pair_hints`、不呼叫 `_format_dedup_hint_block`、不 log `llm_dedup_hints`。因此 `assertNotIn("可能同事件候選", prompts[0])` 對「無論如何都不會出現」的字串做否定斷言，**vacuously true**：即使產品把 hint 誤注入 AF prompt，只要不用到那個確切中文字串，此斷言照樣綠；反過來，它也無法證明「AI 刻意不注入 hint」是有意設計還是碰巧沒寫。此外 fixture 用 OpenAI/Anthropic 兩則零重疊 item，即使有 hint code 也不會產生 pair，等於雙重遮蔽。
`DEDUP_RULES in prompts[0]` 那半是真綁定，故此測不是全廢，但「without tech hints」這個賣點沒被鎖。
**建議**：改成正向鎖 AI 路徑的意圖——(a) 用 fixture 的 1/5（overlap=7）餵進 AI substage，斷言 prompt 仍 **不含** `#1+#5`（證明「AI 即使有同事件候選也不注入 hint」是設計）；或 (b) 若設計上 AI 未來要注入，改測應注入。無論哪個，目前的空集合 fixture 讓斷言測不到契約。

**M2 — T03 未鎖「獨立 pair」的實際內容（`test_dedup_pair_hints_independent_not_transitive`）**
方法名宣稱「A-B、B-C 高重疊但只發獨立 pair、不做 transitive group」，但斷言只有三條結構性檢查：`all(len(p)==3)`、`all(a<b)`、`pairs == sorted(pairs)`。它**沒斷言 pairs 的實際集合**。後果：
- 若實作退化成「只發 (1,2) 漏掉 (2,3)/(1,3)」，三條結構斷言全過（仍是 3-tuple、a<b、有序）。
- 若實作誤做成 transitive 但仍以獨立 tuple 形式輸出（例如把傳遞閉包展開成 pairs），也偵測不到。
「not transitive」這個賣點其實沒被鎖——真正能區分 transitive 與否的是「是否含 (1,3) 這條非相鄰邊」或「component 是否被壓成單一代表」，而測試對這些沉默。
**建議**：加一條 `self.assertEqual(pairs, [(1,2,ov12),(1,3,ov13),(2,3,ov23)])`（或至少 `assertIn((1,3,...))` + 對 overlap 值做 range 斷言）。真值需先跑一次 `_topic_words` 確認三個 overlap 都 ≥4——若某條 <4，改 fixture 標題直到三條相鄰+非相鄰邊都成立，才真正測到「pairwise 全展開 vs transitive 縮併」的差異。

### Minor

**m1 — T20 custom 分支只用 `inspect.getsource` 抓字串，未走執行路徑鎖 variant**
`test_cache_variants_bumped_for_dedup` 對 AI variant 用真常數 `run.AI_SUBSTAGE_CACHE_VARIANT`（好），但 custom 的 `custom_topic_v3_dedup` 是 `_run_custom_topic` 內部區域變數（run.py 2201），只能用 `inspect.getsource(...).__contains__` 抓。這能擋「刪掉字面」的 silent revert，但擋不住「改了 variant 字面但 cache 路徑沒跟上」或「variant 沒實際進 `_news_cache_path`」。T19 已實跑 custom 路徑但**沒斷言 cache 檔名含該 variant**（用 TemporaryDirectory 卻沒檢查落地檔名）。
**建議**：在 T19（已有 tmp cache dir）加一條斷言：run 完後 tmp 內存在含 `custom_topic_v3_dedup` 的 cache 檔（或 patch `_news_cache_path` 捕捉傳入 variant），把「字面存在」升級為「字面被真的用作 cache key」。這才對稱於 AI 側的 `test_clustered_cache_variant_does_not_reuse_legacy_substage_path`（那條有比對真實路徑）。

**m2 — post_dedup trace 的 `before/after/dropped/pairs` 欄位沒被任何測試斷言**
`_post_dedup_selected_summary` 在啟用且有砍時 log `llm_post_dedup` 帶 `before=[...] after=[...] dropped=[...] pairs=[{a,b,overlap}]`（run.py 649–657）。T15/T08 只斷 `enabled` 布林；**沒有任何測試斷 `dropped` / `pairs` 的內容**。對照 hint 側 T09 精確鎖了 pairs shape，post 側的可觀測性契約是空的。營運靠這條 trace 判斷「今天砍了哪些」，值得鎖。
**建議**：在 T10 補抓 `llm_post_dedup` trace，斷 `dropped==[5]`、`before==[1,2,4,5]`、`after==[1,2,4]`、`pairs==[{"a":1,"b":5,"overlap":7}]`。

**m3 — 整合測試把 `_precheck_apply`、`_resolve_paywall_replacements`、`_attach_numbered_links` 全 mock 成 identity/no-op，ordering 斷言依賴 mock 的呼叫順序而非真實副作用**
T16/T17/T18/T19 用 `fake_precheck` 記錄 `precheck_ids` 來證明「precheck 只見 survivors」。這是合理的 spy 手法，但它鎖的是「post_dedup 在傳給 precheck 前已砍」，**不鎖** post_dedup 產出是否真的被下游沿用（因為 `_attach_numbered_links` 也被 mock 成 `(summary, 1)` 直通）。若實作在 post_dedup 後、precheck 前之間某處又把砍掉的 #5 加回，spy 仍顯示 `[[1,2]]` 而最終輸出可能含 #5。T16 有補 `assertNotIn("#5", body)` 補住了 tech 這條，但 **T18/T19 沒有對最終 `lines` 做 `assertNotIn("#5"/"#2")`**，只信 spy。
**建議**：T18/T19 各補一條對最終回傳 `lines` 的 `assertNotIn` survivor-外 marker，讓 ordering 斷言不單靠 spy。

**m4 — 所有 fixture 的 `min_overlap` 依賴預設值 4，只有 T03 顯式傳 `min_overlap=4`**
門檻 4 是散在 `LLM_DEDUP_HINT_OVERLAP` / `LLM_POST_DEDUP_OVERLAP` 兩個常數（run.py 70–71）。測試沒有一條斷言「這兩個常數等於 4」或「hint 與 post 用同一門檻」。若有人把 `LLM_POST_DEDUP_OVERLAP` 改成 3，T11（overlap=3 應保留）會紅——good——但錯誤訊息會指向 post_dedup 邏輯而非門檻常數，且 hint 側 `LLM_DEDUP_HINT_OVERLAP` 若被單獨改動，T02 才會抓到。屬於可接受的間接覆蓋，但一條顯式 `assertEqual(run.LLM_DEDUP_HINT_OVERLAP, run.LLM_POST_DEDUP_OVERLAP, 4)` 能把「兩門檻必須一致」這個註解裡寫的設計意圖（run.py 66–67「Threshold matches hints」）鎖成契約。

### Nit

**n1 — T07 內有一行死碼**：`lines = [f"- #{n} {it['title']}" for n, it in numbered.items()]`（test_run.py 539）算出 `lines` 後從未使用（下一行用另一個 `body` 覆蓋語意）。無害但誤導讀者以為 fake_agent 回的是那個 `lines`。建議刪。

**n2 — T18 fixture 順序註解與斷言耦合較緊**：`items=[title0, title4, title1]` 對應 fixture index 0/4/1，靠註解 `#2 collapsed into #1` 說明。可讀性 OK，但若 fixture 重排會 silently 錯位。非缺陷。

**n3 — 缺 docstring/註解說明 T12 的「first-in-output」與 run.py `_post_dedup_selected_summary` docstring「keep the first bullet in LLM output order」的對應**。測試方法名已足夠自證，屬 nit。

---

## 4. False confidence risks（綠燈但契約未鎖）

1. **【最高】AI substage「不注入 tech hints」未被鎖**（M1）。綠燈只證明「那串中文沒出現」，不證明「AI 有意省略 hint」。若未來把 hint 誤加進 AI prompt 且用不同字串，測試不紅。
2. **「非傳遞」未被鎖**（M2）。T03 綠燈只證明 shape 對，不證明真的展開成獨立相鄰+非相鄰 pair 而非傳遞縮併。
3. **post_dedup 可觀測性契約空白**（m2）。`dropped`/`pairs` trace 若被實作悄悄改欄位名或漏填，無測試會紅——但營運靠它。
4. **custom cache variant「被使用」未鎖**（m1）。字面在源碼裡存在 ≠ 被當成 cache key，兩者間的斷裂 T19/T20 都測不到。
5. **ordering 部分依賴 mock 呼叫序**（m3）。T18/T19 未對最終輸出做 survivor 斷言，理論上存在「spy 顯示已砍但輸出復活」的盲區（T16 已補、T18/T19 未補）。

---

## 5. Missing test cases（建議新增）

- **MC1（對應 M2）**：`_dedup_pair_hints` 對 A-B-C 三連鎖的**完整 pair 集合**斷言（含非相鄰 (1,3) 邊），真正區分 transitive vs independent。
- **MC2（對應 M1）**：AI substage 餵 overlap≥4 的 1/5，斷言 prompt **不含** `#1+#5`（正向證明 AI 刻意不帶 hint），取代目前的空斷言。
- **MC3（對應 m2）**：post_dedup 砍場景斷 `llm_post_dedup` trace 的 `dropped`/`before`/`after`/`pairs`。
- **MC4**：**三則以上同事件 component**（例如 fixture 1/5 再加一個與 1 overlap≥4 的改寫）進 `_post_dedup_selected_summary`，斷言整個 component 只留一則、且留的是 LLM 輸出最先者。目前只測到 2-element component（1+5），未測 ≥3 element 的 union-find 縮併與「跨非相鄰邊」的 component 合併。
- **MC5**：`_dedup_pair_hints` 遇 **空 title / 缺 title key** 的 item（`str(numbered[i].get("title") or "")` 與 `if not wa: continue` 那兩條防線，run.py 544/550）——目前無測試餵空/缺 title，那兩行防禦是 dead-until-proven。
- **MC6（對應 m4）**：`assertEqual(run.LLM_DEDUP_HINT_OVERLAP, run.LLM_POST_DEDUP_OVERLAP)` 且 `== 4`，鎖「兩門檻一致」設計意圖。
- **MC7**：`_post_dedup_selected_summary` 遇 **marker 帶前導破折號變體 / `#N,` 逗號變體**（`marker_re` 的 `(?!,)` 負向斷言，run.py 662）——確認 `#1,` 這種列舉不被誤當 marker 砍。屬邊界但正是 regex 最易錯處。

---

## 6. Ordering / kill-switch / cache 斷言品質

**Ordering（強）**：T16/T17/T18/T19 用 `fake_precheck` spy 捕捉傳入 id，是鎖「A 在 B 前執行」的正解（比事後查輸出更精準）。T17 特別覆蓋 language-fail 分支——這條在 run.py 1746 的 if/else 裡，重構時極易只在 pass 分支保留 post_dedup 而漏掉 fail 分支，測到位。**扣分點**見 m3：T18/T19 未對最終輸出補 survivor 斷言，ordering 保證未閉環。

**Kill-switch（強）**：T08（hints off）、T15（post off）都三面斷——行為（無字串/不砍）+ trace `enabled` 布林 + rules 仍在。T08 額外斷 `DEDUP_RULES` 在 hints off 時仍注入，正確區分「P0 rules 恆在」vs「P1 hints 可關」兩個獨立開關。品質高。**扣分點**：kill-switch 只測「關」，沒測「預設開」——`LLM_DEDUP_HINTS_ENABLED`/`LLM_POST_DEDUP_ENABLED` 的預設值（env 未設時 `!= "0"` → True）無測試守，若有人把預設改成 `== "1"`（未設時變 False）不會紅。建議補一條「env 未設時為 True」的預設守門。

**Cache（中上）**：T20 AI 側用真常數 `run.AI_SUBSTAGE_CACHE_VARIANT` 比對字面 `default_ai_clustered_v5_post_dedup`——擋 silent revert 正確。旁證 `test_clustered_cache_variant_does_not_reuse_legacy_substage_path`（同檔 299–311）已鎖「新舊 variant 路徑必不同」。**扣分點**見 m1：custom 側僅 `inspect.getsource` 抓字面，未鎖「該 variant 真的成為 cache key」，且 T19 有 tmp cache dir 卻沒驗落地檔名。cache 契約在 AI 側閉環、custom 側半開。

---

## 7. Recommended test fixes（按優先序）

1. **修 M1（T07）**：把 AI substage 的「無 hint」斷言從空集合 vacuous 版改為正向版——餵 overlap≥4 的 1/5，斷言 prompt 不含 `#1+#5`。這是目前唯一「測了等於沒測」的斷言，優先修。
2. **補 M2（T03 → 新增 MC1）**：加 pairs 完整集合斷言（含非相鄰邊），讓「not transitive」名副其實。
3. **補 m2（T10 → MC3）**：斷 `llm_post_dedup` trace 的 `dropped/before/after/pairs`，對稱於 hint 側 T09。
4. **補 m1（T19 → MC4 註記的 cache 落地）**：在既有 tmp cache dir 上斷言 custom cache 檔名含 `custom_topic_v3_dedup`。
5. **補 m3（T18/T19）**：對最終 `lines` 加 `assertNotIn` survivor-外 marker，閉環 ordering 保證。
6. **補 MC4**：≥3-element component 的 union-find 縮併測試（目前最大只測 2-element）。
7. **補 MC6 / kill-switch 預設守門**：門檻常數一致性 + env-unset 預設為 True。
8. **清 n1**：刪 T07 死碼行。

（1–3 為進實作審查前建議先補；4–8 可與實作審查併行。）

---

## 8. Ready for implementation review? **Yes（附條件）**

**理由**：核心契約已被真實綁定且全綠——去重門檻（T01/T02/T11）、first-LLM-order union-find（T12）、四條路徑（tech/general/ai/custom）的 rules+hints 注入（T05/T06/T07-partial/T19）、post_dedup-before-precheck ordering 含 language-fail 分支（T16/T17/T18/T19）、雙 kill-switch（T08/T15）、cache variant bump（T20）。這足以支撐實作審查者信任「測試會抓到主要回歸」。

**條件**：M1 讓「AI 不帶 tech hints」這個明確設計賣點實際上未被守住，M2 讓「非傳遞」未被守住。這兩點不影響已實作行為的正確性驗證（那些都綠且真綁），但會讓實作審查者對「這兩個設計約束有測試護欄」產生錯誤信心。建議進實作審查**前先補 §7 第 1–3 項**（約 3 條斷言的小改），或在實作審查時明確標註「T03/T07 為 weak，勿據此認定 non-transitive / AI-no-hints 已鎖」。其餘（4–8）可排入實作審查後的測試補強。
