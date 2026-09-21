# News LLM 去重提案 — Codex design review

## 1. Verdict: approve-with-changes

- **[證據]** tech 目前在進入摘要前只經過 case-insensitive exact-title `dedup()`；AI 才另外走 `cluster()` / `pick_representatives()`。證據位置：`news/scripts/run.py::dedup`（440–449）、`news/scripts/run.py::main`（2258–2261）、`news/scripts/run.py::summarize_llm`（1858–1871）、`news/scripts/run.py::_summarize_default_ai_substaged`（1763–1769）。
- **[證據]** 提案自己的 5-title 數據顯示 #1/#2 overlap 為 3，且共同詞包含 `美光`、`三星`；這正好符合 P1/P2 所提「overlap >= 3 且 entity-like 交集」條件，卻又被提案定義為不同事件。證據位置：`docs/reviews/news-llm-dedup-proposal.md::問題現象`（24–25）、`::測得的 token overlap`（40–47）、`::P1`（91–95）、`::P2`（101–104）。
- **[意見]** 核准問題方向與 P0；P1 必須改成更保守的 soft hint；P2 不應進首批。建議首批為 P0 + tech-only P1（只用 `overlap >= 4`、pairwise、不做 entity lexicon、不做 hard drop）+ 測試/trace/kill-switch。這能處理已知 #1/#5 訊號，同時避免已知 #1/#2 反例被規則直接命中。

## 2. Corroborated facts（path + function + line numbers）

以下均為 **[證據]**；未列為事實的推論留在後續「意見」。

1. `news/scripts/run.py::dedup`（440–449）只以 `item["title"].lower()` 做 exact-title 去重；`news/scripts/run.py::main`（2258–2261）對 `ai`、`tech`、`general` 都先呼叫此函式。因此改寫標題不會被這一層移除。
2. `news/scripts/run.py::_title_without_source`（452–455）移除尾端 ` - Source`；`::_topic_words`（458–474）抽取長度大於 2 的 Latin token 與 CJK bigram，並套用 stopwords / stop bigrams。`_CLUSTER_OVERLAP = 2` 定義於 `news/scripts/run.py`（98–109）。
3. `news/scripts/run.py::cluster`（477–492）將每則標題只和各群的 `seed_words` 比較，交集達 2 即加入，且最後依群大小排序；`::pick_representatives`（495–500）直接取每群前 `per_cluster` 則，不檢查來源或付費狀態。
4. 現有測試明確固定「取群內第一則、未實作免費來源優先」的現況：`news/scripts/test_run.py::NewsClusteringTests.test_pick_representatives_uses_cluster_order_without_source_labels`（375–385）在 WSJ、cnyes、NVIDIA Blog 同群且 WSJ 排第一時預期選 WSJ。
5. 只有 AI 預設路徑在分半前執行 `cluster()` 與 `pick_representatives(..., per_cluster=1)`：`news/scripts/run.py::_summarize_default_ai_substaged`（1754–1775）。路由上，`summarize_llm` 對 `ai` 呼叫該函式，但 `tech` / `general` 呼叫 `_summarize_default_section`：`news/scripts/run.py::summarize_llm`（1858–1871）。`news/SKILL.md::分段策略／AI 預設新聞去重`（79–93）也如此描述。
6. AI 群集已有測試覆蓋 CJK bigram、跨語言共同 token、不做累積詞彙 chain merge、通用 CJK phrase 分離，以及跨 half 去重：`news/scripts/test_run.py::NewsClusteringTests`（323–385）、`::test_summarize_default_ai_no_cross_half_duplicates`（387–434）。
7. tech/general prompt 已含「同事件多來源只挑一則」、免費來源優先、避開付費牆，以及「事件本身」例示規則：`news/scripts/run.py::_summarize_default_section`（1521–1537）。AI substage prompt 含多來源與免費/付費偏好，但沒有最後一條「事件本身」規則：`news/scripts/run.py::_run_ai_substage`（1672–1686）。
8. numbered prompt item 保留 `title`、`link`、`source_name`，並依輸入順序產生 `#N`：`news/scripts/run.py::_number_items_for_prompt`（653–678）。tech 的候選上限是 12、目標選 3–5：`news/scripts/run.py::DEFAULT_SECTION_SPECS`（77–82）；`_summarize_default_section` 實際套用該 limit：`news/scripts/run.py::_summarize_default_section`（1515–1520）。
9. default tech 成功路徑先做 marker validation；語言不合格與合格兩個分支都在 link attach 前做 precheck 與 paywall replacement：`news/scripts/run.py::_summarize_default_section`（1547–1590）。語言失敗分支會以仍存活的 marker IDs 呼叫翻譯：同函式（1566–1581）與 `::_translate_selected_section`（1267–1306）。
10. AI substage 的順序不同：marker validation → precheck → paywall replacement → language validation/translate → link attach：`news/scripts/run.py::_run_ai_substage`（1705–1741）。所以提案的 tech 流程圖不能直接視為所有 default section 的共同流程；提案流程圖位置：`docs/reviews/news-llm-dedup-proposal.md::tech 路徑處理順序`（49–58）。
11. precheck 依 leading marker IDs 決定要檢查的已選項目，並回傳以 marker ID 為 key 的 paywall map：`news/scripts/run.py::_precheck_apply`（893–909、923–936、962–980）。因此任何 selected-set 變更都必須在 `#N` 身分仍存在時完成。
12. AI substage cache 在建 prompt 前即可直接回傳；新結果則在翻譯成功或 link attach 成功後寫入：`news/scripts/run.py::_run_ai_substage`（1653–1667、1722–1741）。目前 variant 是 `default_ai_clustered_v3_precheck`：`news/scripts/run.py::AI_SUBSTAGE_CACHE_VARIANT`（38）。
13. 提案列出的 overlap 數值與本機用現有 `news/scripts/run.py::_topic_words`（458–474）重算相符：#1/#5 = 7、#1/#2 = 3、#1/#3 = 2、#2/#4 = 2；提案原始數值與共同詞見 `docs/reviews/news-llm-dedup-proposal.md::測得的 token overlap`（40–47）。

## 3. Incorrect or risky claims in the proposal

1. **P1 entity 條件和 fixture 自相矛盾。**
   - **[證據]** P1 建議 `overlap >= 3` 且至少一個 entity-like bigram 交集時提示（proposal 91–95）；#1/#2 overlap 正是 3，共同詞含提案點名的 entity-like `美光`、`三星`（proposal 44–45），但需求說兩者應分開（proposal 24–25）。
   - **[意見]** 這不是理論上的 false positive，而是現成 acceptance fixture 會失敗。首版應刪除 entity 分支，只保留 `overlap >= 4` 的 soft pair hint。
2. **P2 使用同一門檻會把已知反例 hard-drop。**
   - **[證據]** P2 建議 `overlap >= 3 且 entity 交集非空，或 overlap >= 4`（proposal 99–104）；#1/#2 滿足前半條件（proposal 44–45）。
   - **[意見]** 「只對已選 3–5 則」雖縮小 blast radius，並沒有提高判斷精度；錯刪會直接損失不同事件，且可能讓 tech 低於 3 則。P2 應從首批 drop，而非僅調插入點。
3. **「candidate groups」若用 connected components，會產生傳遞式誤併。**
   - **[證據]** 提案只示例 `可能同事件候選組: #1+#5`，未定義多 pair 如何成組（proposal 91–97）；同一份 fixture 又要求不得把 1/2/3/5 合成一組（proposal 107–110）。
   - **[意見]** P1 輸出必須是獨立 pair list，不做 union-find / transitive closure；否則一個弱 bridge 可把不同事件串起來。
4. **P2 的「免費源優先」在建議插入點沒有可靠判定資料。**
   - **[證據]** numbered items 只有 `title`、`link`、`source_name`（`news/scripts/run.py::_number_items_for_prompt` 653–678）；真正的 `title_only` / paywall map 是 precheck 後才形成（`::_precheck_apply` 893–909、962–980）。提案卻建議 P2 可在 precheck 前執行並新增 source helper（proposal 101–104）。
   - **[意見]** precheck 前只能靠 publisher 名單猜測；precheck 後才有品質 verdict，但會多做重複的網路檢查。不要為首版 P2 再造第二套來源政策。至於 `lib/news_quality` 可否提供正式 reusable policy，本次依審查範圍未讀，標為 **unverified**。
5. **AI cache bump 條件寫得太窄。**
   - **[證據]** P4 只明說「若 AI prompt 變更」需 bump variant（proposal 113–118）；AI cache 可在 prompt、validation、precheck 前直接回傳（`news/scripts/run.py::_run_ai_substage` 1653–1667）。
   - **[意見]** 任何改變 AI cached output 語意的 P0/P1/P2 都要評估 variant；尤其 marker-based P2 必須在首次 cache write 前完成，並 bump variant，否則舊 cache 完全繞過新行為。
6. **trace schema 與 env semantics 尚未具體化。**
   - **[證據]** 提案只列 trace 名稱與兩個 env（proposal 97、105、113–117），沒有欄位、預設值、invalid value 行為或 cache 互動契約。
   - **[意見]** 首版至少固定 `section`、`candidate_pairs`、pair scores、enabled/disabled 與 before/after（若未來有 hard dedup）；env 解析沿用現有 `"0"` 關閉慣例。P2 未上線就不要先加它的 flag。

## 4. Per-package critique P0–P4（keep | change | drop）

| Package | 決定 | 證據 | 意見 |
|---|---|---|---|
| P0 | **change** | default prompt 已有較完整事件規則，AI prompt 少最後一條事件定義：`news/scripts/run.py::_summarize_default_section`（1528–1537）、`::_run_ai_substage`（1679–1686）。 | 保留抽共用規則與正反例，但例子應寫成可泛化的「同一報告/公告/研究 vs 同產業不同主體事件」，不要把每日 #N 寫死進常數。tech 與 AI 共用；AI 變更同步 bump cache variant。 |
| P1 | **change** | 現有 tech 最多 12 候選：`news/scripts/run.py::DEFAULT_SECTION_SPECS`（77–82）及 `::_summarize_default_section`（1515–1520）；已知 #1/#5 overlap 7、#1/#2 overlap 3（proposal 40–47）。 | 首版只在 tech 啟用；只列 pair、門檻 `>= 4`、不使用 entity lexicon、不傳遞合群；hint 明寫「僅供複核，仍按事件語義判斷」。保留 trace 與單一 kill-switch。 |
| P2 | **drop**（首批） | 提案的 hard rule 會命中應保留的 #1/#2（proposal 24–25、44–45、99–104）；precheck/paywall 身分依 marker IDs：`news/scripts/run.py::_precheck_apply`（893–909、923–980）。 | 在離線標註集證明 precision、定義 under-fill 行為與來源政策前不 ship。未來若重提，只可在 marker 通過後、precheck 前處理，且需保證 cache、翻譯、paywall map 都只看到 survivors。 |
| P3 | **change** | 現有 clustering 測試集中於 AI pre-cluster，未涵蓋提案的 5-title hint/post-select：`news/scripts/test_run.py::NewsClusteringTests`（323–434）。 | 加 5-title 精確 pair assertion、prompt 共用規則、kill-switch、trace、順序與 cache variant 測試；P2 測試列為未來 gate，不隨首批實作。 |
| P4 | **change** | AI 已有 pre-cluster，tech/general 是 single-call：`news/scripts/run.py::summarize_llm`（1858–1871）；現有 SKILL 只文件化 AI deterministic clustering：`news/SKILL.md`（79–93）。 | 上線順序改為 tech P0 → tech P1 soft hints；AI 僅 P0。更新 `SKILL.md`，bump AI cache variant。觀測誤提示後再決定是否擴到 general/AI；不首批上 P2。 |

## 5. Recommended minimal ship set（ordered）

1. **[意見]** 先把 5-title fixture 與 prompt capture 測試寫成紅燈；fixture 的唯一 hint pair 必須是 `(1, 5)`。
2. **[意見]** 抽出共用 dedup prompt 規則，供 `_summarize_default_section` 與 `_run_ai_substage` 使用；規則明確區分「同事件多出口」和「同主題不同事件」。這是 P0。
3. **[意見]** 因 AI prompt 會改，bump `AI_SUBSTAGE_CACHE_VARIANT`；既有 cache variant 行為證據見 `news/scripts/run.py::_run_ai_substage`（1653–1667）及常數（38）。
4. **[意見]** 新增一個純計算的 pair-hint builder（名稱由實作決定），輸入必須是 `_number_items_for_prompt` 產生的 `numbered` 集合，只回傳 `overlap >= 4` 的獨立 pairs。不要 entity lexicon、不要 connected components。
5. **[意見]** 僅在 tech prompt 注入 soft hints，提供 `NEWS_LLM_DEDUP_HINTS=0` kill-switch，並記錄 bounded trace；不改 LLM 選出的 marker set。
6. **[意見]** 更新 `news/SKILL.md`，先 canary 觀測 hints 的 pair precision 與最終重複率。P2、AI P1、general P1 全部延後。

**[意見]** 這個最小組合不增加第二次 LLM call，也不碰 paywall/precheck 資料流；pair 計算只需在有上限的候選集合內做集合交集。**[證據]** tech 原本就是單次 `_summarize_default_section` call（`news/scripts/run.py::summarize_llm` 1866–1871），且 numbered tech candidates 最多 12 個（`::DEFAULT_SECTION_SPECS` 77–82、`::_number_items_for_prompt` 653–678）。

## 6. Tests + acceptance criteria（5-title fixture）

### 固定 fixture

沿用 proposal 的五則標題，事件真值為 `{1,5}` 同事件，`2`、`3`、`4` 各自保留；原始文字與真值證據見 `docs/reviews/news-llm-dedup-proposal.md::問題現象`（16–25）。

### 首批必過測試

1. **topic-word regression**
   - **[驗收/意見]** 用現有 `_topic_words` 斷言 overlap：`1/5=7`、`1/2=3`、`1/3=2`、`2/4=2`；任一 tokenization/stop-list 變更造成數值改變時，測試應要求人工重審門檻，而非默默更新 expectation。
   - **[證據基線]** `_topic_words` 位於 `news/scripts/run.py`（458–474）；proposal 數值位於（40–47）。
2. **hint builder exact result**
   - **[驗收/意見]** threshold 4 時回傳恰好一個 unordered pair `{(1,5)}`；不得包含 `(1,2)`、`(1,3)`、`(2,4)`，也不得輸出 `{1,2,5}` 之類傳遞群組。pair 順序與輸出序列順序必須 deterministic。
3. **prompt contract**
   - **[驗收/意見]** 捕捉 `_summarize_default_section("tech", ...)` 與 `_run_ai_substage(...)` 送出的 prompt，兩者都含同一份 dedup rules；tech prompt 另含 `#1+#5` hint 且不含 `#1+#2`。測試應 mock `_run_nullclaw_agent`，不可呼叫外部 LLM。
   - **[證據基線]** 兩個 prompt 建構點分別在 `news/scripts/run.py`（1521–1537、1672–1686）。
4. **kill-switch + trace**
   - **[驗收/意見]** hints 關閉時 prompt 無 hints、builder 可不執行，trace 明確記 disabled 或不產生 hint event（兩者擇一並固定）；開啟時 trace 只含 `(1,5)` 與 score 7，不含標題全文。
5. **no behavior regression**
   - **[驗收/意見]** 既有 `news/scripts/test_run.py::NewsClusteringTests`（323–434）、marker/language、precheck/paywall tests 全部通過；首批不得更改 LLM output markers、precheck input 或 link attach 結果。
6. **AI cache**
   - **[驗收/意見]** 斷言新 `AI_SUBSTAGE_CACHE_VARIANT` 不重用舊 path；現有對應測試位置為 `news/scripts/test_run.py::test_clustered_cache_variant_does_not_reuse_legacy_substage_path`（299–311），prompt 變更後應更新其 expectation。

### P2 未來重提時的額外 gate（首批不實作）

- **[驗收/意見]** selected markers 為 `#1,#2,#4,#5` 時，只 collapse `1+5`，輸出仍含 `2`、`4`，且代表選擇有明確、單一來源政策。
- **[驗收/意見]** selected markers 為 `#1,#5` 時，規格必須明定是否允許結果低於 tech 最少 3 則；若不允許，需有不增加第二次 LLM call 的 deterministic refill 規則。
- **[驗收/意見]** dedup 後 precheck 只能收到 survivor IDs；paywall replacement、translation、link attach 不得看到被刪 marker；fresh 與 cache-hit 路徑輸出語意一致。

整體 acceptance：上述首批測試全綠、fixture 唯一 hint 為 `(1,5)`、外部 LLM call 數不增加、P2 不在首批 production path。

## 7. Do-not-do list

以下均為 **[意見]**：

1. 不要把 `_CLUSTER_OVERLAP = 2` 的 AI pre-cluster 直接套到 tech；現有 fixture 的 #1/#3 與 #2/#4 都會達 2（proposal 44–47）。
2. 不要首批 ship P2 hard dedup，也不要以「小 N」當成 correctness 證明。
3. 不要使用 `overlap >= 3 + entity`；它已知會命中 #1/#2。
4. 不要建立或維護 entity lexicon 作為此 MVP 的必要條件；別名、跨語言與新公司會持續製造維護成本。
5. 不要把 pair hints 做 transitive closure、connected components 或群組 hard merge。
6. 不要新增第二輪摘要 LLM，也不要讓 pair 計算觸發任何網路或外部 API。
7. 不要在 link attach 後才改 selected set；此時 `#N` 身分已被消耗。marker/precheck/paywall 的證據見 `news/scripts/run.py::_precheck_apply`（893–909、923–980）及 `::_attach_numbered_links`（1003–1046）。
8. 不要在沒有 cache variant 設計的情況下改 AI cached output 語意；cache-hit 會在 prompt 前返回（`news/scripts/run.py::_run_ai_substage` 1653–1667）。
9. 不要複製一套與 content precheck 不一致的 hardcoded free/paywall source policy；本次未讀 `lib/`，可重用性須另行驗證。
10. 不要把 soft hint 的「被提示率」誤當成真實 precision；trace/canary 必須抽樣核對事件語義。

## 8. Answers to open questions 1–6 from the proposal

### 1. P1 門檻在現有 `_topic_words` / stop bigrams 下是否仍會誤提示無關半導體新聞？

- **[證據]** 會。#1/#2 overlap=3 且共同含 `美光`、`三星`，但 proposal 明定兩者不同事件（proposal 24–25、44–45）；這符合原 P1 entity 條件（proposal 91–95）。
- **[意見]** MVP 改為 `overlap >= 4` 的 soft pair hint，拿掉 entity branch。`>=4` 仍不是語義真值，只是這個 fixture 下較安全的提示門檻；需靠 trace/canary 累積 precision。

### 2. P2 是否比把 pre-cluster 擴到 tech 更安全、ROI 更高？

- **[證據]** P2 只碰 LLM 已選的小集合（proposal 99–104），blast radius 確實小於對全部 tech candidates 套 cluster；但它沿用的條件仍會錯判 #1/#2。tech candidates 最多 12、選 3–5（`news/scripts/run.py::DEFAULT_SECTION_SPECS` 77–82）。
- **[意見]** 相對而言 P2 較安全，但尚未安全到可 ship；它的錯誤是 hard deletion，ROI 低於 P0/P1。兩者都不要首批做：不擴 tech pre-cluster，也延後 P2。

### 3. AI substage 已做 pre-cluster，還需要 P2 嗎？

- **[證據]** AI 在分半前已 `cluster` 並每群取一則（`news/scripts/run.py::_summarize_default_ai_substaged` 1763–1769），且現有測試保證已知跨-half duplicate 被移除（`news/scripts/test_run.py::test_summarize_default_ai_no_cross_half_duplicates` 387–434）。
- **[意見]** 首批不需要。AI 先只補 P0 的共同事件規則並 bump cache variant；若 trace 證明 pre-cluster 後仍有顯著漏網，再以獨立 proposal 評估 P1/P2。

### 4. 與 marker validation / paywall replacement / language validation 的插入點衝突？

- **[證據]** P1 在 LLM 前，不碰 output identity，無衝突。若未來做 P2，default tech 最自然位置是 marker validation 通過後、分流 language validation 前（`news/scripts/run.py::_summarize_default_section` 1547–1552）；AI 則是 marker validation 後、precheck 前（`::_run_ai_substage` 1705–1710）。precheck 與 paywall map 都依 marker IDs（`::_precheck_apply` 893–909、923–980）。
- **[意見]** 未來 P2 必須先縮減 summary/selected IDs，再讓 precheck、paywall replacement、translation、attach 全部只處理 survivors；不得在 attach 後做。AI 另須 bump cache variant，確保 cache-hit 回傳的是已套相同規則的結果。

### 5. hint 計算延遲可忽略嗎？entity lexicon 維護成本？

- **[證據]** tech prompt 上限 12 candidates（`news/scripts/run.py::DEFAULT_SECTION_SPECS` 77–82；`::_summarize_default_section` 1515–1520），最多 66 個 unordered pairs；每個 pair 只是 `_topic_words` set intersection（`::_topic_words` 458–474）。
- **[意見]** 在此上限下 CPU 延遲可視為可忽略，但應保留小型 benchmark/trace 避免未來 limit 擴大後失控。entity lexicon 的別名、跨語言、組織改名維護成本不值得 MVP 承擔，因此首版不採用。

### 6. 遺漏 failure mode？更小的最小可行方案？

- **[證據]** 現行資料流存在 cache early return（`news/scripts/run.py::_run_ai_substage` 1653–1667）、marker-based precheck/paywall map（`::_precheck_apply` 893–909、923–980）與 tech 3–5 選取下限（`::DEFAULT_SECTION_SPECS` 77–82），proposal 未完整定義這三者與 P2 的互動（proposal 99–118）。
- **[意見]** 遺漏風險包括：hard dedup 後低於最低則數、pair transitivity、重複/亂序 marker、免費來源政策在 precheck 前不可知、舊 cache 繞過新語意、kill-switch 與 cache 不一致、跨語言 entity 別名漏判，以及高 overlap 的同主體不同事件誤刪。
- **[意見]** 更小 MVP 是：P0 共用 prompt 規則 + AI cache bump；若要直接對本 bug 增加 deterministic 協助，再加 tech-only `overlap >= 4` pair hints、trace、單一 kill-switch。P2、entity lexicon、AI/general hints 都延後。
