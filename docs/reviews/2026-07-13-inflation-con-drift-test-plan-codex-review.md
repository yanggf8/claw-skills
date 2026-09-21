# Inflation-Con Drift Test Plan — Codex Review

## Verdict

The loader unit tests are sound, but the drift plan does not yet prove the intended deployment behavior. T10 and T11 already pass before deployment, T15 stays red only until the gitignore change, and T12 exposes the integration defect only when integration is explicitly forced. These cases must be ordered and asserted so that each red phase demonstrates the intended missing behavior rather than existing behavior or test selection.

## Coverage map T01-T15

| Test | Review status | Evidence / required interpretation |
|---|---|---|
| T01 | Not established | No prior conclusion supplied; retain only if it has a distinct assertion. |
| T02 | Not established | No prior conclusion supplied; retain only if it has a distinct assertion. |
| T03 | Not established | No prior conclusion supplied; retain only if it has a distinct assertion. |
| T04 | Not established | No prior conclusion supplied; retain only if it has a distinct assertion. |
| T05 | Not established | No prior conclusion supplied; retain only if it has a distinct assertion. |
| T06 | Not established | No prior conclusion supplied; retain only if it has a distinct assertion. |
| T07 | Not established | No prior conclusion supplied; retain only if it has a distinct assertion. |
| T08 | Not established | No prior conclusion supplied; retain only if it has a distinct assertion. |
| T09 | Not established | No prior conclusion supplied; retain only if it has a distinct assertion. |
| T10 | Pre-deploy green | Cannot serve as a deployment red test. |
| T11 | Pre-deploy green | Cannot serve as a deployment red test. |
| T12 | Conditional red | Fails only when integration is forced; the plan must force that path explicitly. |
| T13 | Not established | No prior conclusion supplied; retain only if it has a distinct assertion. |
| T14 | Not established | No prior conclusion supplied; retain only if it has a distinct assertion. |
| T15 | Valid narrow red | Remains red only until the gitignore change; it proves ignore coverage, not deployment behavior. |

The loader unit-test layer is covered adequately. Integration and repository-ignore behavior remain separate concerns and should not be presented as one proof.

## Incorrect claims

- Claiming T10 or T11 as red-before-deploy tests is incorrect; both are already green before deployment.
- Claiming T12 reliably catches the integration defect is incorrect unless the test invocation forces integration.
- Claiming T15 validates the deployment is incorrect; its red-to-green transition is caused by the gitignore change.
- Claiming `Path.home()` can be patched after importing the module is incorrect for `DEFAULT_CONFIG`. It is evaluated at import time. Tests must patch home before import or re-bind `DEFAULT_CONFIG` after patching.
- Claiming all T01–T15 provide demonstrated coverage is unsupported by the available evidence; only the conclusions above are established.

## TDD ordering

1. Run the loader unit tests and keep them green as the baseline.
2. Add or invoke T12 with integration forced; observe the integration-specific red before changing deployment behavior.
3. Make the deployment change and require forced-integration T12 to turn green.
4. Run T10 and T11 as regression checks, not as evidence of the red phase.
5. Run T15 before the gitignore change, observe red, then apply only the gitignore change and observe green.
6. Run the minimal suite again from a clean state.

For tests that depend on the default config path, patch `Path.home()` before importing the module. If the module is already imported, explicitly re-bind `DEFAULT_CONFIG` after the patch.

## Minimal test list first

Run these before the full T01–T15 matrix:

1. Loader unit tests.
2. T12 with integration explicitly forced.
3. T15 around the gitignore change.
4. T10 and T11 as pre/post regression checks.

Only expand to T01–T09 and T13–T14 after confirming that each adds a distinct behavior or failure mode.

## Do-not-do

- Do not edit `inflation-con` implementation code while validating this plan.
- Do not use T10 or T11 to claim a red-before-deploy phase.
- Do not run T12 in a mode that silently skips or avoids the forced integration path.
- Do not treat T15 as proof of loader or deployment correctness.
- Do not patch `Path.home()` only after import and assume `DEFAULT_CONFIG` changes automatically.
- Do not combine the deployment and gitignore transitions into one change before recording their independent red/green evidence.
- Do not inflate the minimal suite with tests whose distinct coverage has not been established.
