# News LLM 去重 skips 裁決

日期：2026-07-13

## S1 — 全量 tech/general feed pre-cluster

Verdict: skip

Rationale: `_CLUSTER_OVERLAP=2` 會重現 fixture 1∩3=2 的已知誤併；改成 overlap≥4 又只會提前重做現有 P1/P2 的工作，並在 LLM 判斷前不可逆地縮減候選。排序用途也會讓同主題群集大小干擾新聞重要性。此項永久跳過：tech/general 不做全量 pre-cluster，由現有 P1 soft hints 與 P2 greedy hard dedup 負責。

## S2 — `overlap≥3 + entity lexicon` 門檻

Verdict: skip

Rationale: overlap≥3 加 entity 仍會重現 fixture 1∩2=3 的已知誤連；entity 詞表無法區分「同一實體的不同事件」，且需要持續維護仍不改變判定缺陷。此項永久跳過，不建立 entity lexicon，門檻維持 overlap≥4。

## S3 — underfill 後 refill

Verdict: implement

Rationale: P2 去重後低於 `pick_min` 時，可在不降低門檻、不重跑 LLM 的前提下恢復最低篇數；這不會引入 overlap=2 或 overlap=3+entity 的已知誤併。

Minimal design:

- 僅在 P2 後結果非空且低於 `pick_min` 時執行一次 deterministic refill。
- 依原始 numbered 候選順序掃描未被 LLM 選取的項目；候選與每個已保留項目的 overlap 都小於 4 才可補入，補到 `pick_min` 或候選耗盡為止。
- 不重跑 LLM、不降低 overlap 門檻、不復活被 P2 判定為重複的已選項目。
- trace 記錄 attempted、added、final_count 與仍 underfill 的結果；測試涵蓋可補滿、因 overlap≥4 被拒、候選耗盡三種情況。

## S4 — 程式端免費源 ranking

Verdict: skip

Rationale: 免費源優先是代表新聞的可存取性／編輯排序政策，不是事件去重正確性；把 `source_name` 硬編碼成名單會隨來源付費牆與命名變化失效，也會覆蓋 LLM 已給出的代表順序。此項永久不進入程式端 dedup keep 決策；保留 P0 prompt 的文字偏好即可。

## S5 — AI substage 注入 P1 soft hints

Verdict: implement

Rationale: AI substage 缺少唯一尚未對齊的 P1 層。使用既有 overlap≥4 的獨立 pair hints 只提供 LLM 判斷線索，不做硬合群，也不會引入 overlap=2 或 overlap=3+entity 的已知誤併。

Minimal design:

- 在 AI substage 的 numbered candidates 上呼叫與 default/custom 相同的 P1 hint 產生與 prompt 注入路徑。
- 沿用 overlap≥4、獨立 pair、不傳遞合群、`NEWS_LLM_DEDUP_HINTS` 開關及既有 trace schema。
- 測試確認 AI prompt 在 overlap≥4 時有 hint、overlap=2/3 時無 hint、關閉開關時無 hint。

## S6 — custom-topic 以外路徑遺漏

Verdict: skip

Rationale: 已知的三條路徑是 default tech/general、AI、custom；S5 補上 AI P1 後，三者均具 P0+P1+P2，沒有第四個具體呼叫點可實作。此項作為現有缺口永久跳過；未來若新增入口，必須直接接共用 P0/P1/P2 流程，屬新入口的驗收條件而非本項遺留工作。

## S7 — SKILL.md「刻意不做」措辭

Verdict: implement

Rationale: 操作者規則禁止單方面「刻意不做」；文件必須呈現已裁決的永久邊界及原因，避免把未實作誤寫成任意選擇。

Minimal design:

- 只改 LLM 去重段落的措辭，不改演算法。
- 將「**刻意不做**」改為「**Codex 裁決為永久 skip**」。
- 分別寫清楚：全量 tech/general overlap=2 pre-cluster 因 fixture 1∩3=2 會誤併而 skip；overlap≥3+entity 因 fixture 1∩2=3 會誤連而 skip。
- 同段補充 overlap≥4 pre-cluster 與現有 P1/P2 重複，因此也不另設 pre-cluster。

## SKILL.md 具體改寫指引

把現有末段改成裁決式描述：先用「Codex 裁決為永久 skip」標示權責，再用兩個並列句逐項交代被拒方案、對應 fixture 證據與現行替代機制。不要再使用「刻意不做」、「暫不做」或「之後再看」；結尾明確寫成 tech/general 維持 P1 overlap≥4 soft hints 加 P2 overlap≥4 greedy hard dedup。
