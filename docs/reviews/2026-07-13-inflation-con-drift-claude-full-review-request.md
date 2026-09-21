# Claude full review request (re-run with mandatory tests)

Re-review the completed inflation-con drift fix. **Tests must be run in this session.**

## READ

- inflation-con/scripts/run.py (DEFAULT_CONFIG, load_config, main config load)
- inflation-con/scripts/test_run.py (full file, especially load_config/deploy tests)
- inflation-con/SKILL.md (config section)
- .gitignore (`inflation-con/config.json`)
- docs/reviews/2026-07-13-inflation-con-drift-fix-codex-review.md
- docs/reviews/2026-07-13-inflation-con-drift-test-plan.md
- docs/reviews/REVIEW-PROTOCOL.md

## RUN (mandatory before verdict)

```sh
cd /home/yanggf/a/claw-skills/inflation-con/scripts
python3 -m pytest test_run.py -q --tb=short
INFLATION_CON_REQUIRE_DEPLOY=1 python3 -m pytest test_run.py -q -k live_deploy --tb=short
```

Optional live checks (read-only):

```sh
readlink -f ~/.nullclaw/skills/inflation-con
python3 -c "from pathlib import Path; import runpy; ns=runpy.run_path(str(Path.home()/'.nullclaw/skills/inflation-con/scripts/run.py')); print(ns['load_config'](ns['DEFAULT_CONFIG'])['policy_stance'])"
```

## WRITE

Overwrite or create:

`docs/reviews/2026-07-13-inflation-con-drift-claude-full-review.md`

Sections:

1. Verdict  
2. Scope  
3. **Test run evidence** (commands + exit codes + summary lines — required)  
4. Findings by severity  
5. Deploy correctness  
6. Test quality  
7. Recommended follow-ups  
8. Ready to commit?

Rules: do not modify production code; do not redeploy; Traditional Chinese OK.
