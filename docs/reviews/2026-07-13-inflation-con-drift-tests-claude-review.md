> **歷史紀錄。** 這次審查的對象是 inflation-con 的 Python 測試，那些檔案已於
> 2026-08-02 隨最後一批 Python 刪除。文末未勾的 F1–F5 是當時的審查建議，不是
> 現在的待辦。

# Inflation-Con Drift 測試審查（load_config / deploy-contract）— Claude Review

- 日期：2026-07-13
- 範圍：**僅測試碼**（`inflation-con/scripts/test_run.py`，第 218 行 `# ---- load_config + deploy path contracts` 之後的區塊）
- 對照：測試計畫 `2026-07-13-inflation-con-drift-test-plan.md`、Codex 對計畫的審查 `...-test-plan-codex-review.md`
- 生產碼 `run.py` 僅讀取 `DEFAULT_CONFIG` + `load_config`，**未修改**
- 未執行 deploy。已實際跑過測試子集取得 ground truth（見下）。

---

## 1. 結論（Verdict）

**approve-with-changes（可接受，須先補三處）。**

測試區塊寫得比計畫本身更嚴謹：它把 Codex 對「計畫」提出的兩個主要疑慮（T10/T11 部署前就綠、`Path.home()` import-time 綁定）在「實作」層面已經處理掉了。實測 13 綠、T15 紅（尚未實作 gitignore，符合 TDD 預期）、T12 skip（部署主機未轉 symlink）。

不過仍有幾處**假信心（false confidence）**與**與計畫不一致**的地方需要在進入實作前修正：

1. **T15 斷言太寬** — `"inflation-con/config.json" in gi` 用子字串比對，任何含此片段的行都算過（例：註解 `# do not ignore inflation-con/config.json`）。計畫 T15 明確要求「exact line」。**Major。**
2. **T10 的 `monkeypatch.setattr(Path, "home", ...)` 是死碼** — `load_config` 從頭到尾只用傳入的 `path`，完全不碰 `DEFAULT_CONFIG` 或 `Path.home()`。這個 patch 對被測行為零影響，會誤導讀者以為 T10 在測 home-resolution。**Major（假信心）。**
3. **T14 少測 note 欄位** — 計畫 T14 要求 example「有 `policy_stance` **and note field**」，測試只斷言 `policy_stance`，漏掉 `_policy_stance_note`。**Minor。**

沒有 blocker。可以在補完 1–3 後進實作。

---

## 2. 覆蓋 vs 計畫 T01–T15

| ID | 計畫要求 | 測試函式 | 狀態 | 備註 |
|----|----------|----------|------|------|
| T01 | missing path → `unclear` + 完整 DEFAULT_SERIES | `test_load_config_missing_path_returns_unclear` | ✅ 綠 | 逐鍵比對 series，扎實 |
| T02 | `{}` → stance `unclear` | `test_load_config_empty_object_stance_unclear` | ✅ 綠 | |
| T03 | `restrictive` → 保留 + series 完整 | `test_load_config_restrictive` | ✅ 綠 | 核心保護案例，正確 |
| T04 | `RESTRICTIVE` → strip+lower | `test_load_config_stance_case_normalized` | ✅ 綠 | |
| T05 | `hawkish` → `unclear` | `test_load_config_invalid_stance_becomes_unclear` | ✅ 綠 | |
| T06 | series 部分覆蓋，override 勝出 | `test_load_config_series_partial_override` | ✅ 綠 | 同時驗 override 生效 + 未覆蓋鍵回退 |
| T07 | 未知鍵不崩 | `test_load_config_unknown_keys_ignored_safely` | ✅ 綠 | 額外驗 `extra` 原樣保留 |
| T08 | 路徑存在但 JSON 壞 → **raise**（Codex 修正：不靜默 fallback） | `test_load_config_invalid_json_raises` | ✅ 綠 | 斷言到 `json.JSONDecodeError` 具體型別，好 |
| T09 | `DEFAULT_CONFIG` == home/.nullclaw/... | `test_default_config_path_shape` | ✅ 綠 | |
| T10 | symlink layout → `restrictive`、series 完整 | `test_load_config_via_skills_symlink_layout` | ⚠️ 綠但含死碼 | symlink 有真的走到（`resolve()` 斷言證明），但 `Path.home` patch 無用 — 見 Finding #2 |
| T11 | symlink 但缺 config → `unclear`（要防的 bug） | `test_load_config_symlink_layout_missing_config_unclear` | ✅ 綠 | 已實測：symlink 有被 traverse，缺檔走 `run.py:78` missing 分支 |
| T12 | 部署主機 host-gated 驗收 | `test_live_deploy_loads_restrictive_or_skip` | ⏭️ skip | 有 `INFLATION_CON_REQUIRE_DEPLOY=1` 強制紅路徑，回應了 Codex 的 ordering 疑慮 |
| T13 | chipcon 仍是實體 dir（scope guard） | `test_chipcon_remains_copy_when_present` | ✅ 綠 | |
| T14 | example 有 `policy_stance` **+ note 欄位** | `test_config_example_documents_policy_stance` | ⚠️ 部分 | 漏測 `_policy_stance_note` — 見 Finding #5 |
| T15 | `.gitignore` 含 exact line（實作前紅） | `test_gitignore_scopes_inflation_con_config` | ✅ 紅（符合 TDD） | 斷言用子字串非 exact line — 見 Finding #1 |

**覆蓋整體評語**：T01–T15 全部有對應函式，無缺項。單元載入層（T01–T08）覆蓋充分且斷言具體。路徑解析層（T09–T11）真的走過 symlink（已實測驗證，非讀碼臆測）。額外還多了 `test_load_config_series_partial_override` 對「未覆蓋鍵回退」的雙向驗證，優於計畫最低要求。

---

## 3. 發現（Findings）

### Finding #1 — T15 子字串比對，非計畫要求的 exact line 【Major】
`test_run.py:352`
```python
assert "inflation-con/config.json" in gi
```
計畫 T15 白紙黑字要求「contains **exact line** `inflation-con/config.json`」。目前 `in` 是子字串比對：
- 誤放成 `some-other/inflation-con/config.json.bak` 也會通過；
- 註解行 `# TODO: maybe ignore inflation-con/config.json` 也會通過 → 實作者以為 gitignore 生效但其實 `config.json` 仍會被 commit（正是本次要防的 drift）。

**修正**：改成逐行 exact 比對，例如
```python
lines = gi.splitlines()
assert "inflation-con/config.json" in lines
```

### Finding #2 — T10 的 `Path.home` monkeypatch 是死碼，製造假信心 【Major / false confidence】
`test_run.py:317`
```python
monkeypatch.setattr(Path, "home", classmethod(lambda cls: home))
```
`load_config(path)`（`run.py:77–87`）**只用傳入的 `path`**，從不呼叫 `Path.home()`，也不讀 `DEFAULT_CONFIG`。因此：
- patch `Path.home` 對被測程式碼**零影響**；測試會綠，但綠的原因不是它 patch 的東西。
- 讀者（含未來的實作者）會誤以為 T10 在驗「HOME 解析 → DEFAULT_CONFIG → 讀檔」的完整鏈路。它其實只驗了「把一條已組好的絕對路徑丟進 `load_config`，其穿過 symlink 讀到 repo 的 config」。
- 這正是 Codex 審查第 34 行點名的坑：`DEFAULT_CONFIG` 是 import-time 綁定。測試作者已用第 319 行 `monkeypatch.setattr(run, "DEFAULT_CONFIG", ...)` **正確地** rebind，所以真正生效的是那一行 —— 第 317 行的 `Path.home` patch 是多餘且誤導的殘留。

**修正**：刪掉第 317 行。若想真的驗「import-time 綁定」語意，另寫一個 `importlib.reload` 測試明確涵蓋（optional，非必須）。

### Finding #3 — T12 的 red-before-deploy 只能在部署主機觀測，CI 永遠 skip 【Minor / 需明講】
`test_run.py:362–393`。T12 設計正確（`INFLATION_CON_REQUIRE_DEPLOY=1` 才強制紅，回應 Codex ordering 點 2/3），但這代表：
- 在任何非部署主機（含 CI）上，T12 永遠 skip，**不會**提供 red 證據；
- Codex 的 TDD 步驟 2「observe integration-specific red before changing deployment」只能由「人」在部署主機手動跑一次 `INFLATION_CON_REQUIRE_DEPLOY=1 pytest -k live_deploy` 才成立。

這不是測試 bug，但**實作計畫必須把「部署前在主機上手動觀測 T12 紅」列為一個顯式步驟並記錄輸出**，否則 red/green 證據鏈斷在這裡。屬於 §5 必要事項，不是改測試碼。

### Finding #4 — 沒有測試覆蓋 `main()` 層的壞 config 行為 【Minor】
T08 證明 `load_config` 對壞 JSON 會 raise。但 `main()`（`run.py:306–327`）把 `load_config` 放在 `try` **之外**（第 308 行），壞 config 會直接讓 `main()` 拋例外、**不觸發** `emit_skill_status("failed")`（那個在 try 內的 except 才會呼叫）。也就是說：壞掉的 live config 會讓整個 skill 以未捕捉例外 + exit≠0 收場，且 cron 的 skill-contract trace 不會標記 failed。

這是 Codex「不靜默 fallback」修正的**已知副作用**，方向正確（寧可炸也不要靜默跑 `unclear`），但目前**零測試**描述這條 `main()`-level 契約。建議補一個測試斷言「壞 config → `main()` 回傳非 0 或明確的 failed 狀態」，把契約釘死。屬於覆蓋缺口，非阻擋項。

### Finding #5 — T14 漏測 note 欄位 【Minor】
`test_run.py:355–359`。計畫 T14 要求 example「有 `policy_stance` and note field」。測試只斷言 `policy_stance` ∈ `VALID_STANCES`，漏掉 `_policy_stance_note`。實測 example 確實有該欄位，所以補上即綠：
```python
assert "_policy_stance_note" in example
```

### Finding #6 — T11 docstring 稱「the bug we prevent」但語意需精準 【Nit】
`test_run.py:331`。T11 綠的意義是：symlink 存在但 target 內無 `config.json` 時，loader 回退 `unclear`（**不 raise**）。這確實是「部署忘了放 config」的靜默失效情境，測試抓得對。但要注意它證明的是「missing-file 分支的既有行為」，**不是** deploy 本身正確 —— 這點 Codex 已強調（別拿 T11 當 deploy 紅相證據）。docstring 現有措辭 OK，但若在實作 commit message 引用，別把它講成「證明 symlink 部署成功」。純提醒。

### Finding #7 — `test_load_config_*` 未驗 `series` 深拷貝隔離 【Nit】
`load_config` 用 `dict(DEFAULT_SERIES)` 淺拷貝（`run.py:82`）。value 都是字串，目前無共享可變狀態風險，所以不是 bug。但若未來 series value 變成 dict/list，會出現跨呼叫污染。非本次範圍，僅記錄。

---

## 4. 假信心（False Confidence）

以下是「看起來測到、實際沒測到」的地方，進實作前務必意識到：

1. **T10 給人「HOME→DEFAULT_CONFIG 全鏈路已驗」的錯覺** —— 實際只驗「絕對路徑穿 symlink 讀檔」。`Path.home` patch 是死碼（Finding #2）。真正驗 import-time 綁定坑的是第 319 行的 `DEFAULT_CONFIG` rebind，不是 home patch。
2. **T15 給人「gitignore 規則精確」的錯覺** —— 子字串比對會放行註解與路徑前綴污染（Finding #1）。
3. **T12 給人「部署契約已被自動測試守住」的錯覺** —— 在 CI/非部署主機它永遠 skip，零保護。真正的 red 只在部署主機手動跑才出現（Finding #3）。整套 suite 在 CI 綠燈，**不代表 deploy 已正確轉 symlink**。
4. **整體 suite 綠 ≠ 生產 loader 對壞 live config 會優雅失敗** —— `main()`-level 契約無測試（Finding #4）。T08 只證明 `load_config` 這一層 raise，沒證明上層如何處置。

Codex 對「計畫」的核心疑慮（T10/T11 部署前就綠、`Path.home` import-time）在**測試碼層面已被正確處理**（用 T12 host-gate 承接 deploy 紅相、用 `DEFAULT_CONFIG` rebind 承接 import-time）。上面 4 點是殘留的、Codex 未觸及的假信心點。

---

## 5. 進實作前的必要修正（Required fixes before implement）

**改測試碼（本次審查範圍內，允許）：**
- [ ] **F1**：T15 改為 exact-line 比對（`in gi.splitlines()`），對齊計畫「exact line」要求。
- [ ] **F2**：刪除 T10 第 317 行死碼 `monkeypatch.setattr(Path, "home", ...)`；保留第 319 行 `DEFAULT_CONFIG` rebind。
- [ ] **F5**：T14 補 `assert "_policy_stance_note" in example`，對齊計畫 T14「and note field」。
- [ ] **F4（建議）**：新增一個 `main()`-level 測試，斷言壞 config → 非 0 exit / failed 狀態，把 Codex「不靜默 fallback」修正的上層契約釘死。

**流程/實作計畫（非改測試碼，但必須做）：**
- [ ] **F3**：在實作計畫明列「部署前於主機執行 `INFLATION_CON_REQUIRE_DEPLOY=1 pytest ... -k live_deploy` 觀測 T12 紅 → 轉 symlink → 再跑觀測綠」，並保留兩次輸出作為 red/green 證據（Codex TDD 步驟 2–3）。
- [ ] 依計畫 §4：先加 gitignore line（T15 紅→綠，獨立 commit）→ 再把 live config 複製進 repo + 換 symlink（T03+T10 保護）→ 最後主機驗 T12。三段 red/green 分開記錄，勿合併（Codex do-not-do）。

**不要做：**
- 不改 `run.py` 的 `load_config` / `DEFAULT_CONFIG` 除非上述測試轉紅。
- 不在單元測試裡碰 live deploy 路徑（現有 T12/T13 的 skip/gate 已正確，勿放寬）。
- 不把 deploy 與 gitignore 兩個 transition 併成一次 commit。

---

## 6. 可進實作？（Ready for implementation?）

**Yes — 有條件。**

測試骨架健全、覆蓋 T01–T15 無缺項、TDD 紅相（T15）符合預期、實測 ground truth 與計畫相符。Codex 對計畫的兩大疑慮已在測試層被正確吸收。

進實作前請先套用 §5 的 **F1 / F2 / F5**（三處都是小改、純測試碼、對齊計畫原意），並把 **F3** 的主機端 T12 red/green 觀測列入實作步驟。**F4** 建議補但不阻擋。完成後即可依計畫 §4 的三段式 TDD 推進實作。
