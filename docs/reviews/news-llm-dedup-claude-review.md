# News LLM 去重實作 — Claude code review

- 日期：2026-07-13
- 審查者：Claude（read-only；未改任何業務碼）
- 範圍：P0 `DEDUP_RULES` + P1 soft pair hints + P2 post-select hard dedup
- 對照文件：`news-llm-dedup-proposal.md`、`news-llm-dedup-codex-review.md`、`news-llm-dedup-impl-summary.md`
- 測試現況：`python3 -m pytest news/scripts/test_run.py -q` → **82 passed**（本機重跑確認）

---

## 1. Verdict

**approve-with-changes**

實作正確、測試綠、插入點與 cache/env 契約大致到位；P0/P1 完全符合 Codex 首批建議。**唯一實質問題是 P2 被納入首批**，而 Codex design review 明確把 P2 列為 `drop`（首批不 ship）。P2 現以 `overlap >= 4` + connected-components 實作，帶入一個**可證實的傳遞式誤刪**（transitive over-merge，見 major 一項）與一個**無 refill 的 under-fill 沉默降級**（minor）。兩者都不是 blocker（`overlap>=4` 保守、真實觸發率低、且有 kill-switch），但都與「hard delete 必須先證明 precision」的原則相衝。

可接受的收斂路徑（任一）：
- (A) 首批只留 P0+P1，把 P2 以 `NEWS_LLM_POST_DEDUP=0` 預設關閉上線觀測；或
- (B) 保留 P2 但把 union-find 換成「只砍 pairwise 直接命中的 b（保留 a）」語義，消除傳遞誤刪；並補一條 under-fill trace。

---

## 2. What was reviewed（files + scope）

| 檔案 | 審查焦點 |
|------|----------|
| `news/scripts/run.py` | `DEDUP_RULES`、`_dedup_pair_hints`、`_format_dedup_hint_block`、`_post_dedup_selected_summary`、`_summarize_default_section`、`_run_ai_substage`、`_run_custom_topic`、cache variant 常數 |
| `news/scripts/test_run.py` | `LlmDedupHintsTests`（13 個 case）、其餘 regression suite |
| `news/SKILL.md` | 「LLM 事件去重（完整三層）」段落與 cache/env 描述 |
| `docs/reviews/*` | proposal、Codex review、impl summary（意圖對照） |

方法：逐行讀原始碼、對照 proposal/Codex 意圖、跑既有測試、另寫 scratch 腳本實證 P2 傳遞誤刪（已刪除）。

---

## 3. Findings by severity

### 🟠 Major

**M1 — P2 post-dedup 用 connected-components，會傳遞式誤刪不同事件（`_post_dedup_selected_summary`, run.py:624-647）**

`_post_dedup_selected_summary` 對已選集合先算 pairwise pairs（`_dedup_pair_hints`，只回獨立 pair），**然後用 union-find 把 pair 連成連通分量**，每個分量只留 LLM 順序第一則：

```python
for a, b, _ov in pairs:
    if a in parent and b in parent:
        union(a, b)
```

這正是 Codex do-not-do #5（「不要把 pair hints 做 transitive closure / connected components / 群組 hard merge」）點名的模式，只是搬進了 P2 的 hard-drop 路徑。

**Failure scenario（本機實證）**：三則已選標題 A、B、C，其中 `overlap(A,B)=4`、`overlap(B,C)=4`、`overlap(A,C)=0`。B 是弱 bridge，把兩個**完全無關**（0 交集）的事件 A 與 C 串成同一連通分量：

```
pairs = [(1, 2, 4), (2, 3, 4)]
kept ids = [1]        # #2 與 #3 都被硬刪，即使 #1 與 #3 毫無關係
```

P1（hints）刻意只回獨立 pair、由 LLM 決定，所以不受影響；但 P2 是確定性 hard drop，一旦連通就直接刪 bullet。`overlap>=4` 讓真實觸發率低（要兩個各自 ≥4 的橋接 pair），但**只要出現一個共享大量 bigram 的「綜述型」bridge 標題**（半導體/AI 這類主題常見長共同詞串），就可能一次刪掉兩則不同事件。

**建議**：P2 若保留，改為「只刪 pairwise 命中的較大 marker id（或非代表側），保留 pair 內第一則」，不建連通分量——即對每個 `(a,b)` pair 只在 `a`（keep 側）存活時 drop `b`，不做 union。這保留「同事件改寫只留一則」，同時杜絕 A-C 透過 B 被連坐。或按 verdict (A) 直接把 P2 預設關閉。

---

### 🟡 Minor

**m1 — P2 可把區塊砍到低於 `pick` 下限，無 refill、無 under-fill 告警（`_summarize_default_section`, run.py:1739-1744；`_run_ai_substage`, run.py:1901-1904）**

`spec["pick"]`（tech `3-5`、general `2-3`）只是 prompt 文字，程式從不強制。post-dedup 之後只檢查「是否全空」（空 → `今日無相關新聞` 且 `used_fallback=False`），**不檢查是否掉到下限以下**。若 LLM 選了 3 則、P2 砍成 1 則，digest 就以 1 則 tech 出貨且不告警。Codex open-question #6 與 test-plan P2 gate 都要求「明定是否允許低於最少則數」，實作未回答。屬 minor：`overlap>=4` 保守、空區塊已正確處理、非硬失敗。建議至少加一條 `post_dedup_underfill` trace（before>=pick_min 且 after<pick_min 時），讓 canary 能觀測。

**m2 — hints 與 post-dedup 的 overlap 門檻在同一常數但 trace 未記門檻值（`llm_dedup_hints` / `llm_post_dedup`）**

`LLM_DEDUP_HINT_OVERLAP=4` 與 `LLM_POST_DEDUP_OVERLAP=4` 是兩個獨立常數，目前同值。trace 記了 `pairs`（含每 pair 的 `overlap` 分數）但未記當次生效的 `min_overlap`。日後若有人只調其一，canary log 無法一眼看出門檻。建議在兩個 trace 各加 `min_overlap` 欄位（低成本、純觀測）。

**m3 — 函式內 `import re` 重複（`_post_dedup_selected_summary`, run.py:593）**

`_post_dedup_selected_summary` 內 `import re`，但模組頂層已 `import re`（run.py:6），且檔案已有模組級 `_MARKER_RE`（run.py:1047）。這是既有風格（`_precheck_apply`、`_news_bullet_lines` 等多處都函式內 `import re`），非新引入的問題，但新函式其實可直接用頂層 `re` 或重用 `_MARKER_RE`。純 nit 等級的一致性建議，不影響正確性。

---

### ⚪ Nit

**n1 — post-dedup 的 drop 用新編 `marker_re`，與 `_MARKER_RE`（run.py:1047）語義相同卻各寫一份**

`_post_dedup_selected_summary` 內 `marker_re = re.compile(r"^\s*(?:-\s*)?#(\d+)\b(?!,)")` 與模組級 `_MARKER_RE` 完全一致。重用 `_MARKER_RE` 可少一份需同步維護的 regex。不影響行為。

**n2 — `_format_dedup_hint_block` 的提示文字與 prompt 內既有 `DEDUP_RULES` 有語義重疊**

hint block 說「僅供複核，仍按事件語義判斷；非硬性合併」，`DEDUP_RULES` 也在講同一件事。目前分工清楚（hint 給具體 pair，rules 給判準），不需改；僅記錄以備日後精簡 prompt 長度時參考。

---

## 4. 插入點 / ordering 正確性（marker → post_dedup → precheck → paywall）

**結論：三條路徑的順序都正確，與 Codex 指定的「marker 驗證後、precheck 前」一致。**

| 路徑 | marker 驗證 | P2 post_dedup | precheck | paywall/attach | 位置 |
|------|:-:|:-:|:-:|:-:|------|
| default tech/general | ✅ | ✅（在 precheck 前）| ✅ | ✅ | `_summarize_default_section` run.py:1735-1782 |
| AI substage | ✅ | ✅（在 precheck 前、cache 寫入前）| ✅ | ✅ | `_run_ai_substage` run.py:1896-1937 |
| custom topic | ✅ | ✅（在 precheck 前、cache 寫入前）| ✅ | ✅ | `_run_custom_topic` run.py:2270-2306 |

要點確認：

1. **P2 在 `#N` 身分仍存在時執行**：三條路徑都在 marker validation 通過後、`_precheck_apply` 之前呼叫 `_post_dedup_selected_summary`。precheck 依 leading marker ids 建 paywall map（run.py:1099），post-dedup 先縮 selected set，precheck 只看到 survivors——順序正確，無身分錯位。實測 `test_tech_section_post_dedup_runs_before_precheck`（precheck_ids==[[1,2]]）、`test_ai_substage_post_dedup_before_precheck`（[[1,3]]）、`test_custom_topic_...`（[[1,2]]）三個 case 直接鎖住此不變式。✅
2. **P2 在任何 cache 寫入之前**：AI substage 與 custom topic 都在 `_news_cache_put` / 檔案寫入前完成 post-dedup（run.py:1901 vs 1937；2274 vs 2300），所以 cache 內存的已是去重後 body。配合 variant bump（見 §6），舊 cache 不會繞過新行為。✅
3. **paywall 連續行（`PAYWALL_CONT_PREFIX`）不受影響**：continuation line 是 precheck/attach 階段才產生的，post-dedup 執行時尚不存在，`_news_bullet_lines` 也不會把 continuation 當 bullet（run.py:1174-1175 註解）。無交互 bug。✅
4. **default section 的 language-fail 分支**：語言不合格分支先 precheck、再 `_translate_selected_section`（run.py:1760-1775），P2 已在更早的 run.py:1739 執行過——兩分支共用同一個已去重的 `summary`，正確。✅
5. **空結果處理**：post-dedup 後若 `_news_bullet_lines` 空，三路徑都記 `..._all_dropped`（reason=`post_dedup_empty`）並回 `今日無相關新聞` / 空 lines，`used_fallback=False`，不誤觸發告警。✅（唯 under-fill 未處理，見 m1）

---

## 5. 測試覆蓋評估

**整體：充分（adequate）覆蓋 P0/P1 與 P2 的 happy path 與插入順序；缺 P2 的兩個 correctness gate。**

已覆蓋（`LlmDedupHintsTests`，13 個 case，對照 Codex test-plan）：

- ✅ topic-word overlap regression（`test_fixture_topic_word_overlaps`：1/5=7、1/2=3、1/3=2、2/4=2）
- ✅ hint builder 精確結果（`test_dedup_pair_hints_fixture_only_one_five`：恰 `(1,5,7)`，不含 1/2、1/3、2/4）
- ✅ 獨立 pair 不做傳遞（`test_dedup_pair_hints_independent_not_transitive`）——**注意：這只驗 P1 builder，未驗 P2 的 union-find**
- ✅ format block、tech prompt 含 rules+hint、AI prompt 含 rules 不含 tech hint
- ✅ hints kill-switch + trace（enabled / disabled 兩態）
- ✅ post-dedup 砍 1+5 留 2,4；不砍 overlap=3 的 1+2；post-dedup kill-switch
- ✅ 三路徑 post_dedup 在 precheck 前（tech / AI substage / custom topic）

**缺口（建議補）：**

1. **（對應 M1）P2 傳遞誤刪 gate**：目前**沒有**任何測試餵入「A-B≥4、B-C≥4、A-C<4」的已選三元組去斷言 P2 的連通行為。`test_dedup_pair_hints_independent_not_transitive` 只測 P1 builder 回獨立 pair，**沒測 `_post_dedup_selected_summary` 拿到那些 pair 後會不會 union**。這是最重要的缺口——現有測試讓 union-find 的傳遞誤刪完全隱形。應新增一個 case 斷言期望語義（無論最後選 (A) 或 (B)）。
2. **（對應 m1）under-fill 行為**：Codex test-plan 明列「selected 為 `#1,#5` 時是否允許低於 tech 最少 3 則」為 P2 gate，實作未加對應測試，也未定義行為。
3. **重複 / 亂序 marker**：`_extract_leading_marker_ids` 會去重且保序（run.py:761-774），但 P2 對「LLM 重複輸出同一 #N」或「#N 順序與 numbered 不同」的情況沒有專門測試。低風險（builder 對 subset 重算，順序由 `selected` 決定），但可補一個防迴歸 case。
4. **cache-hit 一致性**：`test_cache_hit_short_circuits_language_gate` 證明 cache-hit 直接回傳（連 language gate 都跳過），意味 **cache-hit 也跳過 P2**。這是正確的（cache 內已是去重後 body，且 variant 已 bump），但沒有一個測試明說「cache body 已含 post-dedup 結果、hit 時不再重跑 P2」。建議補一句註解或斷言，避免日後誤解。

---

## 6. Env / cache 契約檢查

**結論：契約正確、可 kill-switch、cache variant 已正確 bump。**

**Env（沿用既有 `"0"` 關閉慣例，符合 Codex 建議）：**

| 變數 | 預設 | 行為 | 位置 |
|------|------|------|------|
| `NEWS_LLM_DEDUP_HINTS` | on | `=0` → 不注入 hint、跳過 builder，仍記 `llm_dedup_hints{enabled:false}` | run.py:64 |
| `NEWS_LLM_POST_DEDUP` | on | `=0` → P2 不砍，記 `llm_post_dedup{enabled:false}` | run.py:68 |

- ✅ 兩個 flag 各自獨立，可單獨關閉。
- ✅ kill-switch 語義有測試（`test_kill_switch_disables_hints_and_traces`、`test_post_dedup_kill_switch`）。
- ✅ 門檻 `LLM_DEDUP_HINT_OVERLAP=4` / `LLM_POST_DEDUP_OVERLAP=4` 為模組常數，以 keyword-default 綁入函式簽章；測試不 patch 這兩個常數，故 default binding 穩定。
- ⚠️ 門檻未做 env 覆寫（proposal/Codex 也沒要求），且 trace 未記生效門檻（見 m2）。

**Cache variant：**

- ✅ `AI_SUBSTAGE_CACHE_VARIANT`：`default_ai_clustered_v3_precheck` → **`default_ai_clustered_v5_post_dedup`**（run.py:40）。因 P0（AI prompt 加 `DEDUP_RULES`）與 P2（cache body 語義變）都改了 AI cached output，bump 是**必要且正確**的，直接對應 Codex incorrect-claim #5 與 do-not-do #8。舊 variant 的當日 cache 會被忽略。
- ✅ custom topic variant：`custom_topic_v2_precheck`（推斷）→ **`custom_topic_v3_dedup`**（run.py:2201），且 variant 嵌進檔名（run.py:2206），bump 真能讓當日舊 cache 失效（避免 rename no-op）。
- ✅ `test_clustered_cache_variant_does_not_reuse_legacy_substage_path` 從常數推導新檔名，不會因未來再 bump 而失效。
- ✅ SKILL.md 的 cache 段落已更新為 `default_ai_clustered_v5_post_dedup`（SKILL.md:101）。

**SKILL.md 文件契約：** 「LLM 事件去重（完整三層）」段落（SKILL.md:93-99）準確描述 P0/P1/P2 的門檻（`overlap>=4`）、kill-switch、trace 名稱、套用範圍與「刻意不做」清單，與程式一致。唯一未在 SKILL.md 揭露的是 **P2 的 connected-components 語義與 under-fill 行為**——若採 verdict (B) 保留 P2，文件應補一句說明去重是連通分量還是 pairwise，以及是否可能低於 `pick` 下限。

---

## 7. Recommended follow-ups（依序）

1. **（M1，首要）處理 P2 傳遞誤刪**：擇一——(A) `NEWS_LLM_POST_DEDUP` 預設改 `0`，首批只 ship P0+P1（最貼近 Codex 首批建議）；或 (B) 把 `_post_dedup_selected_summary` 的 union-find 改為 pairwise-only drop（每個命中 pair 只砍 b 側、不建連通分量）。
2. **（缺口#1）補 P2 傳遞誤刪測試**：餵 A-B≥4 / B-C≥4 / A-C<4 的已選三元組，斷言最終選出符合所選語義（(A) 則 P2 關閉不砍；(B) 則只砍直接命中的一側）。
3. **（m1 / 缺口#2）under-fill 觀測**：加 `post_dedup_underfill` trace（`before>=pick_min && after<pick_min`），並在 SKILL.md 明定「允許低於下限、不 refill」的決定。
4. **（m2）trace 加 `min_overlap` 欄位** 到 `llm_dedup_hints` 與 `llm_post_dedup`，方便 canary。
5. **（缺口#4）** 補一條測試或註解，鎖定「cache-hit 回傳的 body 已含 P2 結果、hit 時不重跑 P2」。
6. **（n1/m3）** 若動到此函式，順手重用模組級 `_MARKER_RE`、移除函式內 `import re`（純清理，非必要）。
7. Canary 上線後抽樣核對 `llm_post_dedup{dropped}` 的事件語義，確認沒有 §M1 型誤刪，再決定是否把 P2 預設開啟 / 擴到 general。

---

## 8. Do-not-change list（正確地留在外面的東西）

以下是實作**刻意不做**、且我確認**不該改**的：

1. ✅ **不把 `_CLUSTER_OVERLAP=2` 的 AI pre-cluster 全量套到 tech**：fixture #1/#3、#2/#4 都達 2，套了會誤併。實作維持 tech 走 `_summarize_default_section` 單次呼叫，正確。
2. ✅ **不做 `overlap>=3 + entity` 門檻**：已知會命中應保留的 #1/#2。實作統一用 `overlap>=4`、無 entity lexicon，正確（SKILL.md:99「刻意不做」有記）。
3. ✅ **P1 hints 不做 hard drop、不做傳遞合群**：`_dedup_pair_hints` 只回獨立 pair，由 LLM 決定，正確。（注意這條**只適用 P1**；P2 反而違反了傳遞這點——見 M1。）
4. ✅ **不新增第二輪摘要 LLM、pair 計算不觸網**：`_dedup_pair_hints` 純集合交集，無網路/子程序。正確。
5. ✅ **不重造第二套 free/paywall source policy**：P2 代表選擇僅取「LLM 順序第一則」，不自建來源政策（run.py:583-585 註解明說），沿用既有 precheck/paywall 資料流。正確。
6. ✅ **免費源優先仍只是 prompt 文字指引**：`pick_representatives` / P2 都不按 source 排序，維持既有現況（Codex corroborated fact #4）。正確。
7. ✅ **不在 link attach 後改 selected set**：P2 嚴格在 precheck 前、`#N` 身分未消耗時執行。正確。
8. ✅ **AI substage 已有 pre-cluster，未再疊一層 P1/P2 專屬邏輯**：AI 只補 P0 rules + 沿用同一個 `_post_dedup_selected_summary`，符合 Codex「AI 首批只補 P0」的精神（此處多做了 P2，但走的是共用函式，非另造）。

---

## 附註：與 Codex design review 的落差

Codex 對 P2 的裁決是 `drop`（首批不 ship，理由：hard delete 的錯誤是不可逆的事件遺失，且未在離線標註集證明 precision）。本實作**選擇 ship P2**。實作確實吸收了 Codex 的大部分警告——`overlap>=4`、marker→precheck 順序、cache bump、kill-switch——但**沒有吸收「不做 connected components」這一條**，反而在 P2 裡用了 union-find，重新引入了 Codex do-not-do #5 的傳遞誤併。這是本次審查把 verdict 定在 `approve-with-changes` 而非 `approve` 的唯一實質理由。P0 與 P1 我給 clean approve。
