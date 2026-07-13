# Corroboration — test plan Codex review

- Codex: `docs/reviews/2026-07-13-inflation-con-drift-test-plan-codex-review.md`
- Verdict accepted: loader tests OK; clarify red phases

## Applied

| Codex point | Action |
|-------------|--------|
| T10/T11 green before deploy | Documented; used as regression not deploy-red |
| T15 red until gitignore | Kept as intentional fail before implement |
| T12 only red when forced | Added `INFLATION_CON_REQUIRE_DEPLOY=1` hard-fail mode |
| DEFAULT_CONFIG import-time | Tests re-bind `run.DEFAULT_CONFIG` after home patch |

## Current test run (pre-implement)

- Unit/regression: green except **T15 gitignore** (expected red)
- T12: skip (copy) unless REQUIRE_DEPLOY=1 (then red until symlink)
