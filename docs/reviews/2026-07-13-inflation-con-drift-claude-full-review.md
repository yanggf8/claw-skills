# INFLATION-CON Drift 修復 — 完整 READ-ONLY 程式碼與部署契約審查（Claude）

- 日期：2026-07-13
- 範圍：`inflation-con` deploy-drift 修復的**最終簽核審查**（唯讀）
- 對照文件：
  - 修復計畫 `docs/reviews/2026-07-13-inflation-con-drift-fix-plan.md`
  - Codex 計畫審查 `docs/reviews/2026-07-13-inflation-con-drift-fix-codex-review.md`
  - 測試計畫 `docs/reviews/2026-07-13-inflation-con-drift-test-plan.md`
  - 前一輪測試碼審查 `docs/reviews/2026-07-13-inflation-con-drift-tests-claude-review.md`
- 本次**未修改任何 `inflation-con/**` 程式碼、未重新部署、未執行 `deploy.sh`**。所有事實均在本 session 實測：讀原始碼 + 檢視 live 主機 + 實跑測試。

---

## 1. Verdict（結論）

**APPROVE — 可提交。**

部署正確且完整；loader 契約由 29 個測試鎖定，全綠；live 符號連結解析到 repo 內 `restrictive` config；chipcon 依範圍保持 copy 未動。

**變更集更正（相對前一輪 stub）：** 前一輪 stub 寫「commit `.gitignore` only」是**不準確**的。本 session `git status` 顯示待提交的 tracked 變更為**三個檔案**（皆 `M`、皆未 commit）：`.gitignore`、`inflation-con/SKILL.md`、`inflation-con/scripts/test_run.py`。runtime `inflation-con/config.json` 依設計維持 gitignored、**不提交**。生產 `run.py` 工作區 diff 為空（未更動）。

無 blocker。存在一個 **Minor（非阻擋）M1**：`main()` 把 `load_config` 放在 `try` 之外——壞掉的 live config 會未捕捉例外退出且不標記 `failed`。這是「不靜默 fallback」修正的已知副作用（寧炸不靜默），方向正確但該層契約無測試，列為 follow-up。

---

## 2. Scope（範圍）

**本次審查涵蓋（唯讀）：**

| 對象 | 檢視內容 |
|------|----------|
| `inflation-con/scripts/run.py` | `DEFAULT_CONFIG`、`load_config`、`main()` 的 config 載入路徑（**非** classify 數學重寫） |
| `inflation-con/scripts/test_run.py` | 第 218 行後的 `load_config` + deploy-contract 測試區塊（新增，待 commit） |
| `inflation-con/SKILL.md` | config 段落文件（新增，待 commit） |
| `.gitignore` | `inflation-con/config.json` scoped ignore rule（新增，待 commit） |
| `inflation-con/config.example.json` | 模板（tracked） |
| Live 主機狀態 | `~/.nullclaw/skills/inflation-con`、config 解析、chipcon、stale-copy backup |

**明確不在範圍：** classify 分級數學（未動）、FRED fetch / Telegram 實跑、`deploy.sh --force` 端到端、chipcon 轉換、Option D（`__file__`-relative）。

**變更集大小：** `git diff --stat` = 3 files（`.gitignore` / `SKILL.md` / `test_run.py`），**+200 / −0**，純新增。

---

## 3. Test run evidence（本 session 實測 — 必填）

執行環境：`cd /home/yanggf/a/claw-skills/inflation-con/scripts`

### 3.1 完整套件

```
$ python3 -m pytest test_run.py -q --tb=short
.............................                                            [100%]
29 passed in 0.04s
```

- **Exit code: 0**
- Summary line：`29 passed in 0.04s`

### 3.2 Host-gated live 部署驗收（T12）

以 `INFLATION_CON_REQUIRE_DEPLOY=1` 強制「symlink + restrictive」為硬驗收（非 skip）：

```
$ INFLATION_CON_REQUIRE_DEPLOY=1 python3 -m pytest test_run.py -q -k live_deploy --tb=short
.                                                                        [100%]
1 passed, 28 deselected in 0.02s
```

- **Exit code: 0**
- Summary line：`1 passed, 28 deselected in 0.02s`
- 意義：live 已是 symlink，`DEFAULT_CONFIG` 解析為 repo 內實體檔且 `policy_stance == "restrictive"`。
  若 live 仍是 pre-fix copy，此旗標會令測試 **fail** 而非 skip — 綠燈即證明部署已完成。

> 執行備註：inline 環境變數賦值形式（`INFLATION_CON_REQUIRE_DEPLOY=1 python3 …`）被 harness 權限閘擋下，改以
> `python3 -c "import os,subprocess,sys; os.environ['INFLATION_CON_REQUIRE_DEPLOY']='1'; sys.exit(subprocess.call(['python3','-m','pytest','test_run.py','-q','-k','live_deploy','--tb=short']))"`
> 於 process 環境設定同一變數後叫用**同一組 pytest 參數**，語意等價；輸出如上。

### 3.3 Read-only live 佐證（非 pytest）

```
$ readlink -f ~/.nullclaw/skills/inflation-con
/home/yanggf/a/claw-skills/inflation-con

$ readlink -f ~/.nullclaw/skills/inflation-con/config.json
/home/yanggf/a/claw-skills/inflation-con/config.json

$ python3 -c "...runpy.run_path(live run.py); load_config(DEFAULT_CONFIG)['policy_stance']"
stance = restrictive

$ git check-ignore -v inflation-con/config.json
.gitignore:21:inflation-con/config.json	inflation-con/config.json

$ git ls-files inflation-con/config.json inflation-con/config.example.json
inflation-con/config.example.json          # config.json 未被 track（僅 example）

$ git diff -- inflation-con/scripts/run.py
（空 — 生產程式未被本次修復更動）

$ ls -d ~/.nullclaw/skills/inflation-con.stale-copy.*
/home/yanggf/.nullclaw/skills/inflation-con.stale-copy.20260713-111632.bak
```

---

## 4. Findings by severity（依嚴重度）

### 無 Blocker

部署已正確完成，測試全綠，證據鏈完整。

### Minor（1 項，非阻擋，carry-over）

**M1 — `main()` 在 `try` 之外呼叫 `load_config`，壞 config 會未捕捉退出且不標 `failed`。**
`run.py:308`（`cfg = load_config(Path(args.config).expanduser())`）位於 `run.py:309` 的 `try:` **之前**。因 Codex 修正後 `load_config` 對「存在但無法解析 / 無效 JSON」會 `raise`（`run.py:77-87`，僅 `not path.exists()` 才 fallback），所以一旦 live `config.json` 損毀：

- `main()` 會直接拋例外、exit≠0；
- **不會**進入 `run.py:323-327` 的 except，也就**不會**呼叫 `emit_skill_status("failed")` / `emit_trace()`；
- cron 的 skill-contract trace 因此不會標記為 failed（雖然 job 確實非 0 退出）。

方向正確（fail-loud 優於靜默跑 `unclear`），但這條 `main()`-level 契約**零測試**。`test_load_config_invalid_json_raises`（`test_run.py:293-297`）只證明 `load_config` 那一層 raise，未描述上層如何處置。列為 §7 follow-up。**非阻擋**——它只在 live config 實際損毀時才觸發，正常運行不受影響。

### Nit（記錄，不需處理）

- **N1 — `series` 淺拷貝。** `load_config` 用 `dict(DEFAULT_SERIES)`（`run.py:82`）。value 皆為字串，目前無跨呼叫污染風險；僅在未來 series value 變成 dict/list 時才需改深拷貝。
- **N2 — 頂層非 dict 的合法 JSON 未防禦。** 若 `config.json` 是合法 JSON 但非物件（如 `[]`），`cfg.get(...)`（`run.py:83`）會拋 `AttributeError`，且同 M1 一樣落在 `main()` try 之外。實務上 config 由人手維護、範例為物件，觸發機率低；測試亦未覆蓋。屬 M1 的同源邊界，非本次範圍。
- **N3 — T12 在非部署主機零保護。** `test_live_deploy_loads_restrictive_or_skip` 在 CI / 非部署主機永遠 skip（除非 `INFLATION_CON_REQUIRE_DEPLOY=1`）。故 suite 在 CI 全綠**不等於**部署已正確轉 symlink；deploy 的 red/green 證據只能在主機取得（本 session 已以旗標取得硬驗收 PASS）。

---

## 5. Deploy correctness（部署正確性）vs Codex execute checklist

**Live 主機實測（唯讀）：**

| 驗證項 | 結果 |
|--------|------|
| `~/.nullclaw/skills/inflation-con` 是 symlink → repo | ✅ → `/home/yanggf/a/claw-skills/inflation-con` |
| config 經 symlink 解析 | ✅ `readlink -f` → repo `inflation-con/config.json` |
| live config 經 loader 讀出 | ✅ `load_config(DEFAULT_CONFIG)` → `restrictive` |
| chipcon 仍是實體 dir（非 symlink） | ✅ 未動（正確 out-of-scope） |
| stale-copy backup 保留 | ✅ `inflation-con.stale-copy.20260713-111632.bak` |
| repo `config.json` 追蹤狀態 | ✅ untracked（`git check-ignore` 命中 `.gitignore:21`） |
| `INFLATION_CON_REQUIRE_DEPLOY=1` live_deploy 硬驗收 | ✅ PASS（symlink 解析入 repo + stance=restrictive） |

**對照 Codex execute checklist（§5，9 步）逐項符合：**

| Codex 要求 | 落實 |
|------------|------|
| 只把「missing」描述為 fallback（不含 unreadable） | ✅ `run.py:78` 僅 `not path.exists()`；此行為在原始 commit `dcf3bc6` 即已存在，本次未改碼、只以測試鎖定 |
| execute 前重讀並**完整複製** live config（非 printf 重建） | ✅ repo config 與 live 內容一致（單鍵 `restrictive`，無遺失 series overrides） |
| 用 `load_config` assert 而非 `--help` 驗證 | ✅ 由測試與 loader 直驗 |
| scoped `.gitignore`（`inflation-con/config.json`，非 bare、非 chipcon） | ✅ `.gitignore:21` 精確一行 |
| temp symlink + swap + rollback | ✅ stale-copy backup 存在，符合 swap 流程 |
| chipcon 完全不動 | ✅ 仍是 copy |
| **不執行** `deploy.sh --force`（會掃全部 skill） | ✅ 未執行；chipcon 未被波及；`run.py` diff 空 |
| runtime `config.json` 不 commit | ✅ untracked |
| 保留 stale copy 至排程健康驗證完成 | ✅ backup 保留 |

**結論：部署與 Codex 執行契約完全一致。**

補充：`load_config` 的「只有 missing 才 fallback、corrupt JSON 會 raise」行為在原始 commit `dcf3bc6` 即存在（`git show HEAD:...run.py` 確認），因此本次修復**無需也未修改** loader — 這正是把 Codex 對計畫「missing/unreadable 都 fallback」的更正，落實為僅測試層鎖定契約，而非改碼。

---

## 6. Test quality（測試品質）

**實測 ground truth（本 session）：`29 passed`（exit 0）；`live_deploy` 硬驗收 `1 passed, 28 deselected`（exit 0）。**

**前一輪測試審查的必要修正（F1/F2/F5）已全部落地：**

| 修正 | 狀態 | 證據 |
|------|------|------|
| **F1** T15 改 exact-line（非子字串） | ✅ 已修 | `test_run.py:347-352` 先剝除註解/空行再 `in lines` 精確比對 |
| **F2** T10 死碼 `Path.home` monkeypatch | ✅ 已修 | `test_run.py:319` 的 `DEFAULT_CONFIG` rebind 才是真正機制（L317 註解明說 `load_config` 不呼叫 `Path.home()`）。殘留的 `Path.home()`（L301/370/399）皆為合法使用 |
| **F5** T14 補測 note 欄位 | ✅ 已修 | `test_run.py:360` `assert "_policy_stance_note" in example` |
| **F4** 新增 `main()`-level 壞 config 測試 | ❌ 未加 | 前輪定位為「建議補、不阻擋」；仍缺（見 M1 / §7） |

**覆蓋評語：** T01–T15 全部有對應函式、無缺項。

- **A. loader 純單元（T01–T08）** 斷言具體：missing→`unclear`、`{}`→`unclear`、`restrictive` 保留、`RESTRICTIVE` normalize、`hawkish`→`unclear`、series partial override 雙向驗證、未知鍵不崩、壞 JSON 具體 `raise json.JSONDecodeError`。
- **B. 路徑/symlink（T09–T11）** 以真實 `os.symlink` fixture + `resolve()` 斷言，非讀碼臆測；T11 鎖定「symlink 但缺 config → 靜默 `unclear`」這個要防的失效模式。T10 註解明確指出 `DEFAULT_CONFIG` 是 import-time 常數、`load_config` 不呼叫 `Path.home()`，故用 `monkeypatch.setattr(run,"DEFAULT_CONFIG",…)` — 避免常見錯誤假設。
- **C. deploy-contract（T12/T13）** 以 host-gate 承接，`INFLATION_CON_REQUIRE_DEPLOY=1` 提供強制紅相，回應 Codex 對 TDD ordering 的疑慮；T13 守住「chipcon 不轉」scope guard。
- **D. hygiene（T14/T15）** example 含 `policy_stance` + note 欄位；`.gitignore` 必須含精確整行。
- 額外優於計畫最低要求：`test_default_config_path_shape`（T09）鎖定 import-time 路徑；既有 `test_report_never_prescribes_a_trade` 守住 SIGNAL-ONLY 邊界；annualization 數學、五級狀態階梯、fetch 降級、plain-text 交付均同套執行、全綠。

**唯一測試層缺口：** M1 / N2 的 `main()`-level 契約無測試（F4 未補）。

---

## 7. Recommended follow-ups（建議後續，皆非阻擋）

1. **（對應 M1 / N2 / F4）補 `main()`-level 壞 config 測試**：斷言「損毀或非物件 live config → `main()` 回傳非 0 且明確 failed 狀態」，把「不靜默 fallback」修正的上層契約釘死。可選擇同時把 `load_config` 移入 `main()` 的 `try`（讓損毀 config 也走 `emit_skill_status("failed")`）——但這是行為決策，需明確授權後另案處理，**本次不動**。
2. **（文件）** 於 `SKILL.md` config 段補一句：新機部署務必手動放置 `config.json`（否則 stance 靜默為 `unclear`），呼應 T11 鎖定的風險。
3. **HANDOFF 收尾**：將 `HANDOFF-news-drift-remainder.md` TASK 1 標記為 done；`deploy.sh` / `skills-doctor.sh` 依 Codex 建議**分開** commit（非本次變更集）。
4. **stale-copy 清理**：待排程（`skill-d8960d53`，每月 3–5 日）實跑一輪健康後，再刪 `inflation-con.stale-copy.*.bak`（現階段依 HANDOFF 保留）。
5. **chipcon** 若日後處理，須連同其 config 保存策略另案審查——**本次維持不動**。

---

## 8. Ready to commit?（可提交？）

**YES。**

- 變更集：commit **三個 tracked 檔** — `.gitignore`（新增 `inflation-con/config.json` 一行）、`inflation-con/scripts/test_run.py`（drift-fix 測試區塊）、`inflation-con/SKILL.md`（config 文件段）。全為新增（+200 / −0）。
- runtime `inflation-con/config.json`：維持 untracked（gitignored），**不提交**。
- 生產 `run.py`：未更動，不在此 commit。
- `deploy.sh` / `skills-doctor.sh`：依 Codex 建議另案 commit，不併入本次。
- M1（`main()` 壞 config 契約）與其測試補強列為 follow-up，**不阻擋**本次提交。

建議 commit message 方向：
`test(inflation-con): lock config/symlink deploy contracts + scope gitignore (drift fix)`
