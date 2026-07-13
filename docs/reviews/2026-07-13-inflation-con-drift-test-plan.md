# Test plan — inflation-con deploy-drift fix (config + symlink)

- Date: 2026-07-13
- Depends on: Codex execute plan `approve-with-changes`  
  (`docs/reviews/2026-07-13-inflation-con-drift-fix-codex-review.md`)
- Goal: **tests first** so convert-to-symlink cannot silently lose `policy_stance=restrictive`
- Implementation (later): `.gitignore` + copy live config into repo + replace copy with symlink
- **Not in this work**: chipcon conversion; `deploy.sh --force`

---

## 1. Why tests (not only shell checklist)

Deploy is operational, but **behavior is code**:

- `load_config` only fallbacks when path **missing** (not unreadable)
- `DEFAULT_CONFIG` is absolute under `~/.nullclaw/skills/inflation-con/`
- After symlink, that absolute path must resolve to a real file with **restrictive**

Unit/integration tests lock loader contracts; a small symlink fixture locks path resolution. Live deploy is verified by a **read-only** test that can be skipped if not on the deploy host.

---

## 2. Fixture / truth

| Source | Expected |
|--------|----------|
| Live pre-fix (2026-07-13) | `~/.nullclaw/skills/inflation-con` is **real dir**; `config.json` = `{"policy_stance":"restrictive"}` |
| Post-fix | same path is **symlink** → repo; `config.json` via symlink still `restrictive` |
| Repo | `config.example.json` remains tracked; `config.json` **gitignored**, not committed |

---

## 3. Test inventory

### A. `load_config` pure unit (tmp files; no network)

| ID | Test | Assert |
|----|------|--------|
| T01 | missing path | returns `policy_stance=="unclear"` and full `DEFAULT_SERIES` keys |
| T02 | empty object `{}` | stance normalizes to `unclear` (default field) |
| T03 | `{"policy_stance":"restrictive"}` | stance `restrictive`; series keys still full defaults |
| T04 | `{"policy_stance":"RESTRICTIVE"}` | strip+lower → `restrictive` |
| T05 | invalid stance e.g. `hawkish` | → `unclear` |
| T06 | series override partial | merges into DEFAULT_SERIES; override key wins |
| T07 | extra unknown keys | does not crash; still returns usable series+stance |
| T08 | path exists but invalid JSON | **raises** (documents Codex correction: no silent fallback) |

### B. `DEFAULT_CONFIG` + symlink resolution (tmp home layout)

| ID | Test | Assert |
|----|------|--------|
| T09 | `DEFAULT_CONFIG` equals `Path.home() / ".nullclaw/skills/inflation-con/config.json"` | exact path |
| T10 | Fake layout: `tmp_home/.nullclaw/skills/inflation-con` → symlink to `tmp_repo/inflation-con`; write config in repo dir; `load_config` via expanded DEFAULT_CONFIG under patched HOME | stance `restrictive`; series intact |
| T11 | Same layout **without** config.json in target | `load_config` → `unclear` (proves missing-file risk if deploy forgets config) |

Implementation note: use `monkeypatch.setenv("HOME", tmp)` or patch `Path.home` if used; run.py uses `Path.home()`.

### C. Deploy contract (optional, host-gated)

| ID | Test | Assert |
|----|------|--------|
| T12 | If `~/.nullclaw/skills/inflation-con` exists: if symlink, `readlink -f` under repo; `load_config(DEFAULT_CONFIG)` stance is `restrictive` **or** document skip if host not deployed | marks live acceptance |
| T13 | chipcon still real dir (not symlink) when host has chipcon | scope guard |

T12/T13: `pytest.mark.integration` or skip if paths absent — never mutate deploy.

### D. Docs / hygiene (lightweight)

| ID | Test | Assert |
|----|------|--------|
| T14 | `config.example.json` has `policy_stance` and note field | example remains template |
| T15 | `.gitignore` contains exact line `inflation-con/config.json` **after implement** | red until implement; TDD intentional |

---

## 4. Implementation mapping (tests → work)

| After tests green require | Implementation |
|---------------------------|----------------|
| T15 | add gitignore line |
| T03 + T10 | place runtime config in repo + symlink deploy |
| T12 | live verification post-deploy |

No change to `classify` math unless tests fail (out of scope).

---

## 5. Explicit non-tests

- Full FRED fetch / Telegram delivery  
- `deploy.sh --force` end-to-end  
- chipcon conversion  
- Option D `__file__`-relative DEFAULT_CONFIG (unless Codex reopens)

---

## 6. Codex review contract (this plan)

Write: `docs/reviews/2026-07-13-inflation-con-drift-test-plan-codex-review.md`

Sections:

1. Verdict: approve | approve-with-changes | reject  
2. Coverage map T01–T15  
3. Missing / wrong tests  
4. Ordering (TDD): which tests must fail before deploy, which only after  
5. Execute-safe notes (no live mutation in unit tests)  
6. Do-not-do  

Read-only; do not implement.
