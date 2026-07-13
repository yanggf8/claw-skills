# Plan: finish inflation-con deploy-drift fix

- Date: 2026-07-13
- Context: HANDOFF from 2026-07-08 (`HANDOFF-news-drift-remainder.md`)
- Status: **proposal only** — wait for Codex review + corroboration before execute
- Out of scope (unless Codex says otherwise): **chipcon** (user previously scoped out; leave as COPY)

---

## 1. What `config.json` is (for this skill)

`inflation-con` is a signal skill (FRED series → RED / YELLOW / GREEN style status).

**`config.json` is the skill’s local runtime config**, not secrets and not shared CLAW_CONFIG.

| Field | Role |
|-------|------|
| `policy_stance` | **Manual human input**: FOMC policy stance after each meeting. One of `restrictive` \| `neutral` \| `easing` \| `unclear`. **Never machine-parsed from FOMC text.** |
| `series` (optional) | FRED series id map; defaults embedded in code if omitted. |

Loader (`inflation-con/scripts/run.py`):

```python
DEFAULT_CONFIG = Path.home() / ".nullclaw" / "skills" / "inflation-con" / "config.json"
```

- Path is **absolute under `~/.nullclaw/skills/inflation-con/`**.
- If that path is a **symlink into the repo**, the file must exist **inside the repo skill dir** (or the lookup fails open).
- Missing / unreadable file → fallback `policy_stance: "unclear"` (silent behavior change).

**Live host today:**

```json
{"policy_stance": "restrictive"}
```

at `~/.nullclaw/skills/inflation-con/config.json` (real directory copy).

**Repo today:** only `config.example.json` with `"policy_stance": "unclear"` — no `config.json`.

**Why it matters for RED:** classifier needs “context not easing”; `policy_stance != "easing"` contributes. If config vanishes and stance falls back to `unclear`, RED context logic still can fire, but Telegram text and operator cues change, and HANDOFF documents intended live stance as **restrictive**.

---

## 2. Current drift state (verified 2026-07-13)

| Skill | Deploy type | Notes |
|-------|-------------|--------|
| news | OK symlink → repo | Fixed 2026-07-08; same inode as repo |
| cct2 | OK symlink → repo | Fixed 2026-07-08 |
| inflation-con | **DRIFT copy** | HANDOFF TASK 1 unfinished |
| chipcon | **DRIFT copy** | Out of scope (user); WOULD-LOSE `config.json` |
| deploy.sh / skills-doctor.sh | Untracked in repo | Written 2026-07-08; not committed |

`skills-doctor.sh`: `2 drift` = inflation-con + chipcon.

---

## 3. Recommended fix (option A)

Preserve live stance **restrictive**, convert inflation-con to symlink, avoid git noise.

### Steps

1. **Backup live tree**  
   `TS=$(date +%Y%m%d-%H%M%S)`  
   `cp -a ~/.nullclaw/skills/inflation-con/config.json /tmp/inflation-con-config.$TS.json`  
   (value must remain `{"policy_stance":"restrictive"}`)

2. **Gitignore runtime config (scoped)**  
   Append to repo `.gitignore`:
   ```
   inflation-con/config.json
   ```
   Optionally also `chipcon/config.json` (same pattern). **Do not** use bare `config.json` globally.

3. **Write config into repo skill dir**  
   ```bash
   printf '%s\n' '{"policy_stance": "restrictive"}' > /home/yanggf/a/claw-skills/inflation-con/config.json
   ```
   Confirm: `git check-ignore -v inflation-con/config.json` matches; `git status` does **not** list it as untracked.

4. **Replace deploy copy with symlink**  
   ```bash
   cd ~/.nullclaw/skills
   mv inflation-con inflation-con.stale-copy.$TS.bak
   ln -s /home/yanggf/a/claw-skills/inflation-con inflation-con
   ```

5. **Verify**
   - `readlink -f ~/.nullclaw/skills/inflation-con` → repo path  
   - `readlink -f ~/.nullclaw/skills/inflation-con/config.json` →  
     `/home/yanggf/a/claw-skills/inflation-con/config.json`  
   - `cat` that file → `restrictive`  
   - `python3 ~/.nullclaw/skills/inflation-con/scripts/run.py --help` (or dry run if available)  
   - Confirm output / log shows `stance=restrictive` (not `unclear`)  
   - `dash skills-doctor.sh` → inflation-con **OK**; only chipcon remains DRIFT  

6. **Docs / hygiene (same PR or follow-up)**  
   - Commit: `.gitignore` change only (not `config.json` content)  
   - Optionally commit `deploy.sh` + `skills-doctor.sh` as deploy tooling  
   - Update or archive `HANDOFF-news-drift-remainder.md` as done for TASK 1  
   - Leave **chipcon** as copy unless user reopens scope  

### Explicit non-goals

- Do **not** convert chipcon without a separate plan (config + WOULD-LOSE).  
- Do **not** change `run.py` path resolution unless Codex requires it (symlink+repo config is enough).  
- Do **not** put real `policy_stance` into git-tracked files.

---

## 4. Alternatives (for Codex to accept/reject)

| Option | Summary | Tradeoff |
|--------|---------|----------|
| **A (recommended)** | gitignore + repo-side config.json + symlink | Clean; matches HANDOFF |
| **B** | repo config.json untracked, no gitignore | Works; `git status` noise |
| **C** | leave as COPY | No risk; perpetual DRIFT |
| **D (optional code)** | resolve config via `Path(__file__).resolve().parents[1] / "config.json"` | Survives layout changes; needs tests + still need config file |

---

## 5. Questions for Codex

1. Approve option A as the execute plan? Any GO-WITH-FIXES?  
2. Should we also gitignore `chipcon/config.json` for consistency (without converting chipcon)?  
3. Commit `deploy.sh` + `skills-doctor.sh` in the same change set or separate?  
4. Is absolute `DEFAULT_CONFIG` under `~/.nullclaw/skills/...` still OK long-term, or prefer `__file__`-relative (option D)?  
5. Any extra verification beyond skills-doctor + stance string?

---

## 6. Codex output contract

Write: `docs/reviews/2026-07-13-inflation-con-drift-fix-codex-review.md`

Sections:

1. Verdict: `approve` \| `approve-with-changes` \| `reject`  
2. Corroborated facts (path/line)  
3. Incorrect / risky claims in the plan  
4. Answers to questions 1–5  
5. Execute checklist (ordered, copy-pasteable)  
6. Do-not-do list  

Do **not** execute deploy steps; do **not** edit skill code unless asked later.
