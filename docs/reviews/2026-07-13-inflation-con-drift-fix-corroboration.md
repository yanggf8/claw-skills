# Grok corroboration — inflation-con drift fix plan (Codex review)

- Codex: `docs/reviews/2026-07-13-inflation-con-drift-fix-codex-review.md`
- Plan: `docs/reviews/2026-07-13-inflation-con-drift-fix-plan.md`
- Verdict: **approve-with-changes** — **採納**

## Corroborated

| Codex 斷言 | 核對 | 結果 |
|------------|------|------|
| DEFAULT_CONFIG absolute path L50 | 是 | PASS |
| load_config 僅 `not exists` → unclear | L77–79 無 try/except | PASS |
| stance 合法集 + normalize | L65, 85–86 | PASS |
| RED: only `easing` 使 not-easing clause 失敗 | L146 附近 | PASS（restrictive vs unclear 分類布林相同） |
| live config `{"policy_stance":"restrictive"}` 實體目錄 | 本機 2026-07-13 | PASS |
| `--help` 不 load config | parse_args 先於 load_config | PASS |
| deploy.sh --force 會掃全部 skill | 計劃勿用 | PASS（意見） |

## 採納的執行修正（相對原計劃 A）

1. 不要寫「unreadable 也 fallback」——只有 missing  
2. 驗證用 `load_config(DEFAULT_CONFIG)` assert stance，**不用** `--help`  
3. **完整 cp** live `config.json`，不要 printf 只寫 stance  
4. symlink 用 temp + swap + rollback  
5. 不 gitignore chipcon；不 commit runtime config；不跑 `deploy.sh --force`  

## 尚未執行

部署步驟未跑；等你說 execute。
