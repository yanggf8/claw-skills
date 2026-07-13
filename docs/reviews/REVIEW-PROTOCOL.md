# Review protocol (claw-skills)

## Claude code review — mandatory tests

When requesting a Claude Code review of **implementation and/or tests**, the review
prompt **MUST** require Claude to:

1. **Run the relevant test suite** (not claim green from memory or prior logs).
2. **Record exact commands and exit status** in the review file.
3. **Paste or summarize the last lines of test output** (e.g. `N passed in …`).
4. If tests fail, **verdict cannot be approve**; list failures under Findings.

Suggested prompt block (copy into every Claude review task):

```
<verification_loop>
You MUST run tests before writing the verdict:
  cd <skill>/scripts && python3 -m pytest test_run.py -q --tb=short
  (or the suite named in the request)
If deploy acceptance is in scope:
  INFLATION_CON_REQUIRE_DEPLOY=1 python3 -m pytest … -k live_deploy -q
Record: command, exit code, summary line (e.g. "29 passed").
Do not approve if you did not run tests in this session.
</verification_loop>
```

Codex design/test-plan reviews remain read-only unless the request says otherwise.
