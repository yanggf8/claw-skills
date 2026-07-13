<!-- TASK 1 STATUS: DONE 2026-07-13 — inflation-con is symlink; config restrictive preserved; see docs/reviews/2026-07-13-inflation-con-drift-* -->

# HANDOFF — finish the nullclaw skill deploy-drift remediation

You are picking up work already ~80% done in another session. Read this fully, then
finish the two remaining tasks. Everything below is verified against the live host;
trust it but re-check before any destructive step.

## Background (already DONE — do NOT redo)
Root cause: `~/.nullclaw/skills/<name>` dirs are supposed to be SYMLINKS into the
repo `/home/yanggf/a/claw-skills`, but several were stale standalone COPIES running
old code. The `news` skill copy predated the paywall feature → a paywalled Washington
Post item shipped with no 付費牆 note. Fixed by converting the copy to a symlink.

Confirmed complete:
- `news`  → symlink to repo. VERIFIED working (paywall note + apnews replacement +
  footer render; traces show quality_tier1/quality_tier2/paywall_notice). Its old
  copy is backed up at `~/.nullclaw/skills/news.stale-copy.20260708-161449.bak`.
- `cct2`  → symlink to repo (Codex GO: repo supersedes the diverged run.py; its
  config.json is byte-identical to repo; CLAW_CONFIG is unset on this host so the
  repo's `os.environ.get("CLAW_CONFIG") or ~/.nullclaw/config.json` falls back to the
  same path the old copy hardcoded). Old copy backed up at
  `~/.nullclaw/skills/cct2.stale-copy.<ts>.bak`.
- Two new repo files written + adversarially verified (GO) by a workflow, already on
  disk (untracked): `/home/yanggf/a/claw-skills/deploy.sh` (idempotent symlink
  deployer, refuses to clobber copies without --force+backup) and
  `/home/yanggf/a/claw-skills/skills-doctor.sh` (read-only drift audit, always exit 0).
  Run `dash /home/yanggf/a/claw-skills/skills-doctor.sh` any time to see current state.

Out of scope (do NOT touch): `chipcon` (user scoped it out — leave it a COPY),
`autocli` (no repo dir here), `cct` / `ainews` (symlinks to OTHER repos).

## TASK 1 — Convert `inflation-con` copy → symlink, PRESERVING its config

The trap: `inflation-con/scripts/run.py` reads config from a FIXED ABSOLUTE path:
```python
DEFAULT_CONFIG = Path.home() / ".nullclaw" / "skills" / "inflation-con" / "config.json"
```
(line ~50). After symlinking the dir to the repo, that absolute path resolves THROUGH
the symlink into the repo dir — which has only `config.example.json`, NO `config.json`.
The skill's loader falls back to `policy_stance: "unclear"` when the file is missing
(run.py ~line 79), silently changing behavior from the intended `"restrictive"`.

Facts:
- Live config to preserve: `~/.nullclaw/skills/inflation-con/config.json` =
  `{"policy_stance": "restrictive"}`
- Deployed `run.py` is IDENTICAL to repo (no code drift; conversion is purely for
  convention + future-drift prevention).
- Repo has `config.example.json` (default stance "unclear"), not `config.json`.
- `config.json` is NOT gitignored in this repo; `chipcon/config.json` sets the
  precedent of an untracked runtime config living in a repo skill dir.

Decision needed (pick one — the previous session paused HERE to ask the user; if the
user has since answered, honor that; otherwise these are the options, recommendation
first):
  (A) RECOMMENDED: add a `config.json` ignore rule to `/home/yanggf/a/claw-skills/.gitignore`,
      then place `{"policy_stance":"restrictive"}` at
      `/home/yanggf/a/claw-skills/inflation-con/config.json`. Clean: absolute lookup
      resolves, no behavior change, no git noise. NOTE: a broad `config.json` ignore
      would ALSO stop tracking any real config.json elsewhere — scope the rule to
      `inflation-con/config.json` (and, if you like, `chipcon/config.json`) rather
      than a bare global `config.json`.
  (B) Same placement but NO gitignore rule → config.json shows as untracked (matches
      chipcon). Zero .gitignore change.
  (C) Leave inflation-con as a COPY (skip). Its code is already current, so drift risk
      is low; only forgoes the convention.

Procedure for (A)/(B):
  1. `TS=$(date +%Y%m%d-%H%M%S)`
  2. Stash the value: keep a copy of the live config.json somewhere safe first.
  3. If (A): append `inflation-con/config.json` (and optionally `chipcon/config.json`)
     to `.gitignore`. Verify with `git check-ignore inflation-con/config.json`.
  4. Write `{"policy_stance": "restrictive"}` to
     `/home/yanggf/a/claw-skills/inflation-con/config.json`.
  5. `cd ~/.nullclaw/skills && mv inflation-con inflation-con.stale-copy.$TS.bak`
  6. `ln -s /home/yanggf/a/claw-skills/inflation-con ~/.nullclaw/skills/inflation-con`
  7. VERIFY: `python3 ~/.nullclaw/skills/inflation-con/scripts/run.py --help` runs, and
     confirm the config resolves — e.g. a dry/normal run shows stance=restrictive, NOT
     unclear. Check `readlink -f ~/.nullclaw/skills/inflation-con/config.json` points at
     the repo file you wrote.
  8. Re-run `dash /home/yanggf/a/claw-skills/skills-doctor.sh` — inflation-con should
     now be OK (symlink), only chipcon should remain DRIFT.

## TASK 2 — News proxy verification (STEP 3/4 — prove deployed news is current code)

Run a cron-EQUIVALENT news proxy to STDOUT ONLY (no --deliver-to → nothing sent to any
user) and assert the deployed code is the current v3 build. This is cache-safe per a
prior Codex review: a same-day AI cache HIT skips quality_tier2 but emits
`news_cache_hit` with variant `default_ai_clustered_v3_precheck`, so accept EITHER.

```sh
JOB_ID="proxy-news-$(date +%Y%m%d-%H%M%S)-$$"
# Taiwan-keyed cache dir; clearing today's forces a fresh compute so quality_tier2 fires.
# If host TZ != Asia/Taipei this may target the wrong day — harmless (v3 cache_hit still PASSes).
rm -rf "$HOME/.nullclaw/.news-cache/$(TZ=Asia/Taipei date +%Y-%m-%d)"
NULLCLAW_JOB_ID="$JOB_ID" timeout 900 python3 "$HOME/.nullclaw/skills/news/scripts/run.py" ; echo "[proxy exit=$?]"

TF="$HOME/.nullclaw/skill-traces.jsonl"
T1=$(jq -c --arg j "$JOB_ID" 'select(.job_id==$j and .skill=="news" and .event=="quality_tier1")' "$TF" | head -1)
T2=$(jq -c --arg j "$JOB_ID" 'select(.job_id==$j and .skill=="news" and .event=="quality_tier2")' "$TF" | head -1)
CH=$(jq -c --arg j "$JOB_ID" 'select(.job_id==$j and .skill=="news" and .event=="news_cache_hit" and .variant=="default_ai_clustered_v3_precheck")' "$TF" | head -1)
V2=$(jq -c --arg j "$JOB_ID" 'select(.job_id==$j and .skill=="news") | select((.variant? // "")|test("clustered_v2"))' "$TF")
printf 'tier1: %s\ntier2: %s\ncachev3: %s\nv2(bad): %s\n' "${T1:-MISSING}" "${T2:-none}" "${CH:-none}" "${V2:-none}"
[ -n "$T1" ] && { [ -n "$T2" ] || [ -n "$CH" ]; } && [ -z "$V2" ] \
  && echo "PASS: deployed news is CURRENT (v3)" \
  || echo "FAIL: staleness/degraded"
```

Also eyeball the stdout digest for a `⚠️ 付費牆（原文需訂閱）` note / `ℹ️ 本次含 N 則付費牆新聞`
footer if any paywalled item was picked (confirms the paywall path is live).

CAVEATS (do not skip):
- This fake-job-id proxy proves only the DEFAULT-path news code is current v3. It does
  NOT gate deleting the stale backup — only a GENUINE SCHEDULED cron fire (real
  scheduler job_id `skill-<uuid>:<n>` passing the same tier1 + (tier2 OR v3 cache_hit)
  + no-v2 check) authorizes deleting `news.stale-copy.20260708-161449.bak`.
- The two `--account-topics` news crons (accounts ping/nunu) use a DIFFERENT code path
  and are not exercised by this default-path proxy.

## TASK 3 — Backups + report
- Do NOT delete any `*.stale-copy.*.bak` yet (news, cct2, inflation-con) — they're the
  rollback net until a genuine scheduled cron run is confirmed healthy.
- Leave `deploy.sh` / `skills-doctor.sh` untracked; commit only if the user asks (repo
  convention: git commit messages end with the Co-Authored-By trailer).
- Final report to the user: what got converted, TASK 2 PASS/FAIL with the trace lines,
  and the remaining follow-ups (confirm next real scheduled cron; then delete backups;
  chipcon still a copy by design).

## HARD CONTRACT (must hold throughout)
nullclaw skills exit 0 on upstream errors and never break delivery. skills-doctor is
read-only + always exit 0. deploy.sh never runs a skill and is not a cron gate. The
proxy omits --deliver-to so it cannot message a real user. Nothing here may abort a
news cron delivery.
