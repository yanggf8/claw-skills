# Inflation-con deploy-drift 修復計畫 Codex 審查

## 1. Verdict: approve-with-changes

採用 Option A，但執行前必須修正三點：把「missing / unreadable 都會 fallback」改成只有「路徑不存在」才 fallback；不可用 `--help` 當作 config 載入驗證；部署時應複製並比對完整 live `config.json`，不要重建成只含 `policy_stance` 的新檔。另應維持 `chipcon` 完全不動，且本次不要執行會遍歷全部 skill 的 `deploy.sh --force`。

## 2. Corroborated facts (with file:line citations from actual code)

- `DEFAULT_CONFIG` 不是設定字典，而是一個固定的預設路徑：`Path.home() / ".nullclaw" / "skills" / "inflation-con" / "config.json"`；CLI 另可用 `--config` 覆寫，載入前會做 `expanduser()`（`inflation-con/scripts/run.py:50`, `inflation-con/scripts/run.py:68-74`, `inflation-con/scripts/run.py:306-311`）。
- 路徑不存在時，`load_config` 回傳完整 `DEFAULT_SERIES` 副本與 `policy_stance: "unclear"`；路徑存在時則讀 JSON、把使用者的 `series` 疊加到內建預設，並正規化 stance（`inflation-con/scripts/run.py:55-65`, `inflation-con/scripts/run.py:77-87`）。
- 合法 stance 只有 `restrictive`、`neutral`、`easing`、`unclear`；缺值或非法值都會正規化為 `unclear`（`inflation-con/scripts/run.py:65`, `inflation-con/scripts/run.py:85-86`）。
- `policy_stance` 是人工輸入而非從 Fed 文字解析；程式的模組說明與範例設定都如此定義（`inflation-con/scripts/run.py:15-17`, `inflation-con/config.example.json:11-12`）。
- stance 只參與 RED 的 context clause：必須先有「10Y breakeven 至少 2.5，或約三個月呈上升」，再加上 `policy_stance != "easing"`；RED 同時還要求 core PCE 3m/6m 均至少 3.5 與 core CPI 確認（`inflation-con/scripts/run.py:131-146`, `inflation-con/scripts/run.py:151-164`）。因此 `restrictive`、`neutral`、`unclear` 在這個布林判斷中完全相同，只有 `easing` 會使此 clause 為 false；不過 stance 原值會出現在訊息與 record 輸出（`inflation-con/scripts/run.py:146`, `inflation-con/scripts/run.py:207`, `inflation-con/scripts/run.py:245-276`, `inflation-con/scripts/run.py:279-289`）。
- 範例檔除 stance 外也列出七個 `series` override，另含一個純說明欄位 `_policy_stance_note`（`inflation-con/config.example.json:1-12`）；現有 loader 不會拒絕未知欄位，但執行路徑實際取用的是 `cfg["series"]` 與 `cfg["policy_stance"]`（`inflation-con/scripts/run.py:77-87`, `inflation-con/scripts/run.py:306-320`）。
- HANDOFF TASK 1 明確要求保留 live 的 `restrictive`、指出 repo 當時只有 example，並把 scoped ignore + repo-side runtime config 列為 A 案（`HANDOFF-news-drift-remainder.md:31-64`）。本次唯讀觀察也與該紀錄一致：live 路徑是實體目錄、live config 為 `{"policy_stance": "restrictive"}`，且 live/repo `run.py` 的 SHA-256 相同；這些是 2026-07-13 的主機狀態觀察，不是程式保證。
- 現有 `.gitignore` 只有一般產物與 credential patterns，沒有 `inflation-con/config.json` 規則（`.gitignore:1-18`）。
- `skills-doctor.sh` 對 symlink 的 OK 判定只檢查其 real path 是否落在 repo；它不讀 config，也不檢查 stance（`skills-doctor.sh:72-83`, `skills-doctor.sh:125-133`）。
- `deploy.sh --force` 對每個符合條件的實體部署目錄都會備份後換成 symlink，主迴圈會遍歷 repo 中每個含 `SKILL.md` 的 immediate subdirectory；它沒有單一 skill selector（`deploy.sh:33-54`, `deploy.sh:143-169`, `deploy.sh:199-206`）。

## 3. Incorrect or risky claims in the plan

1. 計畫的「Missing / unreadable file → fallback」不正確（`docs/reviews/2026-07-13-inflation-con-drift-fix-plan.md:27-30`）。只有 `not path.exists()` 會 fallback；存在但無法開啟或 JSON 無效時，`load_config` 沒有 `try/except`（`inflation-con/scripts/run.py:77-87`）。更重要的是 `load_config` 發生在 `main()` 的 `try` 之前，所以這類錯誤不會走既有的 graceful failure handler（`inflation-con/scripts/run.py:306-327`）。
2. 「fallback 到 `unclear` 會改變 RED 行為」需要限縮。由 `restrictive` 變成 `unclear` 不會改變 RED 的真假，因為兩者都滿足 `!= "easing"`；會變的是輸出的 stance、reason 與人工操作提示（`inflation-con/scripts/run.py:146-164`, `inflation-con/scripts/run.py:256-275`, `inflation-con/scripts/run.py:283-289`）。若計畫所稱「behavior」包含可觀察文字，則成立；若指分類結果，則不成立。
3. 「`config.json` 不是 secrets」無法由程式碼保證（`docs/reviews/2026-07-13-inflation-con-drift-fix-plan.md:10-19`）。目前範例只含 series、stance 與說明文字（`inflation-con/config.example.json:1-12`），但 loader 接受一般 JSON object 且沒有 schema 或敏感欄位防護（`inflation-con/scripts/run.py:77-87`）。應描述為「目前觀察到的內容不是 credential」，不要把它提升為長期安全契約。
4. 以 `printf` 重建只有 stance 的檔案有競態與資料遺失風險（`docs/reviews/2026-07-13-inflation-con-drift-fix-plan.md:77-81`）。即使今日 live 檔只有 stance，執行日前可能已加入合法的 `series` overrides；loader 確實會使用這些 overrides（`inflation-con/scripts/run.py:82-86`, `inflation-con/scripts/run.py:306-311`）。應在 execute 當下重新驗證並原樣複製完整檔案。
5. `python3 .../run.py --help` 不能證明 config resolves 或 stance 為 `restrictive`（`docs/reviews/2026-07-13-inflation-con-drift-fix-plan.md:90-97`）。`parse_args()` 在 `load_config()` 之前執行，而 argparse 的 help 會直接結束該次呼叫；真正載入在後續的 `load_config(Path(args.config).expanduser())`（`inflation-con/scripts/run.py:68-87`, `inflation-con/scripts/run.py:306-311`）。
6. 原計畫的兩步 `mv` / `ln -s` 缺少第二步失敗時的就地 rollback（`docs/reviews/2026-07-13-inflation-con-drift-fix-plan.md:83-88`）。這是操作風險，不是已發生的程式缺陷；應先建立並驗證 temporary symlink，再 swap，失敗則立即把 stale copy 移回。
7. Option D 的「survives layout changes」過度寬泛（`docs/reviews/2026-07-13-inflation-con-drift-fix-plan.md:113-120`）。它只會把耦合從固定 home deploy path 改成 resolved source-tree path；是否更合適取決於預期部署拓樸。現有 CLI 已有 `--config` override（`inflation-con/scripts/run.py:68-74`, `inflation-con/scripts/run.py:306-311`），所以 D 不是完成 A 的必要條件。

## 4. Answers to plan questions 1–5

1. **Option A：approve-with-changes。** 必要修正是：只把 missing 描述為 fallback；execute 前重讀並完整複製 live config；加入 loader-only 驗證；用有 rollback 的 scoped swap；保留 stale copy。A 符合 HANDOFF 的首選方案（`HANDOFF-news-drift-remainder.md:51-80`），且不需改 `run.py`。
2. **不要順手 gitignore `chipcon/config.json`。** HANDOFF 明確把 `chipcon` 排除在範圍外（`HANDOFF-news-drift-remainder.md:28-29`），本次需要的最小規則只有 `inflation-con/config.json`。若日後處理 chipcon，應連同其 deploy/config 保存策略另案審查。
3. **`deploy.sh` 與 `skills-doctor.sh` 分開處理、分開 commit。** HANDOFF 本身要求除非使用者另行要求，先保持兩者 untracked（`HANDOFF-news-drift-remainder.md:118-125`）；而且 `deploy.sh` 的影響範圍是所有 repo skills，不是這次單一 inflation-con（`deploy.sh:143-169`, `deploy.sh:199-206`）。本次變更集只需要 scoped `.gitignore` 規則與本 review/handoff 狀態文件；runtime `config.json` 不提交。
4. **現行 absolute `DEFAULT_CONFIG` 對目前 symlink deploy convention 可接受，Option D 不應阻擋此次修復。** 固定路徑是程式明示的 default，且可用 `--config` 覆寫（`inflation-con/scripts/run.py:50`, `inflation-con/scripts/run.py:68-74`）。若長期目標包含從任意 checkout 直接執行或更換 deploy root，才建議另案評估 `__file__`-relative，並補 missing、invalid JSON、CLI override、symlink/direct-run 測試；這是架構建議，不是由目前程式可證明的需求。
5. **需要額外驗證。** 除了 `skills-doctor` 與輸出 stance，至少要驗證：(a) repo runtime config 與 execute 當下 live 原檔 byte-identical；(b) `readlink -f` 同時確認 skill root 與 config；(c) 直接呼叫 `load_config(DEFAULT_CONFIG)` 並 assert stance，而不啟動 FRED fetch 或 delivery；(d) `git check-ignore -v` 命中精確 scoped rule；(e) chipcon 仍是未改動的實體 copy；(f) 失敗時 rollback。`skills-doctor` 單獨不足，因為它只分類 link target（`skills-doctor.sh:72-83`）。

## 5. Execute checklist (ordered, safe)

以下僅供獲得 execute 授權後執行；本次 review 不執行任何一行。

1. 在同一個 shell 建立路徑與 timestamp，確認 live 仍是實體目錄而非 symlink：

   ```sh
   set -eu
   REPO=/home/yanggf/a/claw-skills
   LIVE="$HOME/.nullclaw/skills/inflation-con"
   TS=$(date +%Y%m%d-%H%M%S)
   test -d "$LIVE"
   test ! -L "$LIVE"
   test -f "$LIVE/config.json"
   ```

2. 在 execute 當下驗證完整 JSON 與 stance，不假設 2026-07-13 的觀察仍未變：

   ```sh
   python3 - "$LIVE/config.json" <<'PY'
   import json, sys
   with open(sys.argv[1], encoding="utf-8") as f:
       cfg = json.load(f)
   assert isinstance(cfg, dict)
   assert cfg.get("policy_stance") == "restrictive"
   print("validated live config: stance=restrictive")
   PY
   ```

3. 以受限權限備份 config，並 byte-compare：

   ```sh
   umask 077
   CFG_BAK="/tmp/inflation-con-config.$TS.json"
   cp -a "$LIVE/config.json" "$CFG_BAK"
   cmp -s "$LIVE/config.json" "$CFG_BAK"
   ```

4. 只在 repo `.gitignore` 增加精確規則 `inflation-con/config.json`，不要加 bare `config.json` 或 chipcon 規則；接著確認規則生效。現有檔案尚無該規則（`.gitignore:1-18`）。

   ```sh
   cd "$REPO"
   grep -qxF 'inflation-con/config.json' .gitignore || \
     printf '\ninflation-con/config.json\n' >> .gitignore
   git check-ignore -v inflation-con/config.json
   ```

5. 原樣複製 execute 當下的完整 live config 到 repo，避免遺失 loader 支援的 `series` overrides（`inflation-con/scripts/run.py:82-86`）：

   ```sh
   cp -p "$LIVE/config.json" "$REPO/inflation-con/config.json"
   chmod 600 "$REPO/inflation-con/config.json"
   cmp -s "$LIVE/config.json" "$REPO/inflation-con/config.json"
   git check-ignore -v inflation-con/config.json
   ```

6. 先在同一目錄建立 temporary symlink 並驗證 target，再進行 scoped swap；第二個 `mv` 失敗時立即 rollback：

   ```sh
   NEW="$HOME/.nullclaw/skills/.inflation-con.new.$TS"
   BAK="$LIVE.stale-copy.$TS.bak"
   test ! -e "$NEW" && test ! -L "$NEW"
   test ! -e "$BAK" && test ! -L "$BAK"
   ln -s "$REPO/inflation-con" "$NEW"
   test "$(readlink -f "$NEW")" = "$REPO/inflation-con"
   mv -- "$LIVE" "$BAK"
   if ! mv -- "$NEW" "$LIVE"; then
     mv -- "$BAK" "$LIVE"
     exit 1
   fi
   ```

7. 不執行 `main()`、不 fetch、不 delivery，直接驗證 default path 與 loader 結果。`main()` 的真正資料抓取在 config 載入之後（`inflation-con/scripts/run.py:306-320`），而 `__main__` guard 在檔尾（`inflation-con/scripts/run.py:330-331`）：

   ```sh
   PYTHONDONTWRITEBYTECODE=1 python3 - "$LIVE/scripts/run.py" <<'PY'
   from pathlib import Path
   import runpy, sys
   ns = runpy.run_path(sys.argv[1])
   expected = Path.home() / ".nullclaw" / "skills" / "inflation-con" / "config.json"
   assert ns["DEFAULT_CONFIG"] == expected
   cfg = ns["load_config"](expected)
   assert cfg["policy_stance"] == "restrictive", cfg
   print(f"loader OK: {expected} stance={cfg['policy_stance']}")
   PY
   ```

8. 驗證 link、config、doctor 與 scope；`skills-doctor` 是 warning-only、固定 exit 0（`skills-doctor.sh:125-133`），所以仍須人工看輸出內容：

   ```sh
   test "$(readlink -f "$LIVE")" = "$REPO/inflation-con"
   test "$(readlink -f "$LIVE/config.json")" = "$REPO/inflation-con/config.json"
   test -d "$HOME/.nullclaw/skills/chipcon"
   test ! -L "$HOME/.nullclaw/skills/chipcon"
   dash "$REPO/skills-doctor.sh"
   git -C "$REPO" diff -- .gitignore
   git -C "$REPO" status --short -- .gitignore inflation-con/config.json
   ```

9. 任一驗證失敗時，不刪除任何內容：把失敗 link 移到保留名稱，再把 `$BAK` 移回 `$LIVE`；成功時也保留 `$BAK` 與 `/tmp` config backup，直到真實排程健康驗證完成。HANDOFF 同樣要求暫不刪 stale copies（`HANDOFF-news-drift-remainder.md:118-125`）。

## 6. Do-not-do list

- 本次 review 不執行部署、不執行上述 checklist、不修改 `inflation-con/**`、不碰 `~/.nullclaw/**`，也不執行 `mv` 或 `ln`。
- 未另行授權前，不執行 `deploy.sh --force`；它會遍歷所有 repo skills，可能連 out-of-scope 的 chipcon copy 一併替換（`deploy.sh:143-169`, `deploy.sh:199-206`）。
- 不加入 bare `config.json` 全域 ignore，不順手加入 `chipcon/config.json`，不提交 runtime `inflation-con/config.json`。
- 不把 `--help`、單獨的 `skills-doctor` 結果或「輸出中看見 stance 字串」當成 config 載入的充分證明；loader 應直接 assert（`inflation-con/scripts/run.py:68-87`, `skills-doctor.sh:72-83`）。
- 不把 live config 重建為僅含目前已知欄位；完整複製可保留合法的 series overrides（`inflation-con/scripts/run.py:82-86`）。
- 不修改 `run.py` 為 Option D，不轉換 chipcon，不刪除任何 `*.stale-copy.*.bak`（`HANDOFF-news-drift-remainder.md:28-29`, `HANDOFF-news-drift-remainder.md:118-125`）。
- 不執行正常 FRED/delivery run 作為本次 config 驗證；`main()` 會進入 fetch 與可能的 delivery/record 路徑（`inflation-con/scripts/run.py:306-327`）。

## 7. What config.json is

`config.json` 是 inflation-con 預設從 `~/.nullclaw/skills/inflation-con/config.json` 讀取的本機 runtime JSON，也可由 `--config` 改用其他路徑；程式實際使用可選的 `series` overrides（與七個內建 defaults 合併）及人工維護的 `policy_stance`，後者會正規化為四個合法值之一，且在分類邏輯中只影響 RED 的 not-easing context clause（`inflation-con/scripts/run.py:50-65`, `inflation-con/scripts/run.py:68-87`, `inflation-con/scripts/run.py:131-164`, `inflation-con/scripts/run.py:306-311`）。因此計畫對「local runtime config、manual stance、series optional」的描述正確；但「不是 secrets」只能描述目前觀察到的範例/live 內容，不能由無 schema、無敏感欄位限制的 loader 保證（`inflation-con/config.example.json:1-12`, `inflation-con/scripts/run.py:77-87`）。

註：`docs/reviews/news-llm-dedup-work.log` 是錯誤的 append 位置；不要寫入該 log。
