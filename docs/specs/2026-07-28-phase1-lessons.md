# Phase ① lessons — read before starting Phase ②

Phase ① (claw-core + doughcon) is implemented and committed. This file records what was learned by actually executing the plan, so Phases ②–④ do not rediscover it. **The shipped code under `crates/` is the reference implementation — where a plan code block and the shipped source disagree, the shipped source is right.**

The single most useful finding: **a fully green suite proved nothing.** The first implementation passed 59/59 Rust tests, 15/15 Python tests, and 7/7 differential cases while containing two real behaviour bugs and four holes that made those tests unable to detect them. Three adversarial verifiers found them; one returned `suite_actually_green: false` against the other two.

---

## 1. Test anti-patterns that produced green-but-worthless suites

**Shared mutable temp files across tests in one binary.** Every test wrote the same `/tmp/claw-core-tg-<pid>.json` via `File::create`, which truncates. Cargo runs tests in one binary on multiple threads, so a reader could land inside the truncation window, get an empty file, resolve "no token", and return `false` with zero HTTP attempts. Three different tests were observed failing nondeterministically.
→ **One file per call**, not per process. Use an `AtomicUsize` counter in the name.

**A stub server that fails open.** The scripted-response stub answered `200` once its script was exhausted. That converts "the client retried when it must not" into a passing test — the worst possible failure mode for a test double.
→ **Fail closed.** Answer something no script uses and that the code under test treats as terminal (we use `418`).

**A shutdown channel that can never fire.** `rx.try_recv().is_ok()` is false for *both* `Empty` and `Disconnected`, so dropping the sender never broke the accept loop and every stub leaked a thread and a socket.
→ Match `Ok(_) | Err(TryRecvError::Disconnected)`.

**Env-var mutation without a lock.** Documenting `--test-threads=1` does not enforce it; a plain `cargo test` still races. A `static ENV_LOCK: Mutex<()>` taken by *every* test in the file is the only sound fix — and it must be **every** test, not just the new one. Phase ① shipped a lock that exactly one of six tests acquired, and the flake was observed for real.

**Tests that re-implement the logic they claim to pin.** The Python `IndexDerivationTests` copied `run.py`'s derivation into the test and asserted against its own copy. They passed against a deliberately broken `run.py`. They were the declared behaviour oracle and pinned nothing.
→ A characterization test must **execute the subject**. Drive `main()` and assert on real output.

**Assertions that never see the composition.** `budget` and `telegram` were each unit-tested, but nothing proved `deliver()` passes the resolved deadline through. Likewise `parse_mode` is pinned on `SendOptions` but not through `deliver()` — dropping it there still passes everything.
→ For every A→B plumbing, write one test that can only pass if the value actually crosses the boundary.

**Uniform fixtures hiding a quantifier.** Swapping `.all()` for `.any()` survived the whole suite because every fixture was uniformly null or uniformly non-null. Only a **mixed** list distinguishes them.
→ When the logic is a quantifier, a fixture that exercises the mixed case is mandatory.

## 2. The verification method that actually worked

Run three verifiers with **different lenses**, not three copies of the same review:

1. **parity** — diff observable behaviour against the frozen oracle, branch by branch.
2. **tests-can-fail** — deliberately break the implementation, confirm the *right* test goes red, revert. This is the only technique that found the worthless tests.
3. **runs-clean** — build and run everything for real, including deliberately *without* the documented flags, to see whether the suite is sound or merely lucky.

Require **quoted code or real command output** as evidence for every claim. Unevidenced findings are noise.

## 3. Language and toolchain facts, verified by compiling

- **Zig 0.16 has no `std.posix.clock_gettime`.** Probe before writing; this repo's own working pattern is in `src/compat.zig:274`.
- Inside `cron.zig`, module functions are called **unqualified** — there is no `cron_mod` self-import (all 164 existing tests do this).
- `try` will not compile inside `fn runQueueWorker(...) void` — no error union. Use the surrounding `catch |err| {...}` style.
- **Python binds default arguments at definition time.** `trace_marker.emit_skill_status(status, stream=sys.stdout)` captures the *original* stdout, so `contextlib.redirect_stdout` cannot see the marker lines. Capture at the **file-descriptor** level.
- **Python's `float()` strips surrounding whitespace; Rust's `f64::from_str` does not.** `trim()` before parsing env values.
- **`IFS=$'\t' read` collapses runs of tabs**, because tab is IFS whitespace. An intentionally empty TSV column is swallowed and every later column shifts. The `no_job_id` case silently tested something else entirely.
- **`set -e` aborts a shell script before its own `FAIL:` branch can run** when the failing command is a plain invocation. A "we refuse bad input" path needs a probe that reaches the check.
- `ureq` 2: `200 → Ok(resp)`; `4xx/5xx → Err(Error::Status(code, resp))`; connection failure → `Err(Error::Transport)`. **A 204 arrives as `Ok`**, so success must be `status() == 200`, never `is_ok()`.
- `jiff` `%Z` yields the DST-correct abbreviation (`EDT`/`EST`). Do not hand-roll it from an offset. A timezone-naive timestamp fails to parse.
- Cargo **hard-errors** on a workspace member whose directory does not exist, so a staged build must not list a member before creating it.

## 4. Python semantics that a Rust port gets wrong by default

These bit Phase ① in both directions — first by narrowing too much, then by over-correcting.

| Python | Naive Rust | Correct |
|---|---|---|
| `x == 0` is true for `0`, `0.0`, `-0.0`, **`False`** | matching only integer `0` | carry the `== 0` answer alongside the rendered value |
| `d.get("k", "?")` defaults **only on an absent key** | a catch-all arm that also swallows `null`/`bool` | distinguish absent from present-but-null |
| f-strings render `True`/`False` capitalised, floats with a decimal point | `serde_json`'s `to_string()` for bools | render bools explicitly |
| `p.get("k") is None` — only null counts | `as_f64()` returning `None` on a non-numeric value | ask "is it JSON null or absent", not "does it parse" |

The general rule: **model what the Python actually uses**, not what its values look like. `run.py` does exactly two things with `overall_index` — renders it and compares `== 0` — so the Rust type carries exactly those two things.

## 5. Sequencing rules that held up

- Agents must **not commit**. Every defect above was found while the tree was uncommitted, and the history stayed clean.
- Tasks sharing a crate's `lib.rs` and `Cargo.toml` must run **sequentially**. Independent tracks (a different repo, a different language) parallelise safely.
- A concurrent-edit race was still observed: one agent's differential run executed against a `main.rs` that another agent was mid-write on. **Re-run any whole-system check after the tree settles** — do not trust a result produced during a write window.

## 5b. Found while executing Phase ② — the installer was never generic

`tools/install-skill.sh` advertises `install-skill.sh <skill-name>` but its
smoke test was hardcoded to **doughcon's** argument surface
(`--mode record --et-hour 99`). Phase ① never ran it against a second skill, so
the assumption held until `weather` arrived and was refused outright. The strict
refusal was correct; the check was not.

The generic replacement: feed a flag **no skill defines** and require the binary
to load and reject it through its own parser (exit 2). That proves the artifact
executes and its argument handling runs, without performing a real invocation —
`weather` would make live HTTP calls, which an install step must not do.

**And then `set -e` bit again.** The probe is *expected* to exit non-zero, so
running it bare aborted the script before its own check could execute — the
exact failure this document already records in section 3. Writing the lesson
down did not prevent repeating it; putting the probe inside an `if` (where
`set -e` is suspended) did. Assume any deliberately-failing command in a
`set -e` script needs that treatment.

## 6. Still open at the end of Phase ①

Recorded so Phase ② can decide whether to close them:

- `crates/doughcon/src/main.rs` has **no `cargo test` coverage** — the DST gate, exit codes, arg parsing and `format_updated` are reachable only through the bash differential harness.
- The `CLAW_CONFIG` branch of `resolve_config_path` and the `parse_mode` plumbing through `deliver()` are unasserted; deleting either passes the suite.
- A `data` array of scalars makes Python raise outside its `try` (exit 1, no markers) where Rust degrades cleanly to `ok`. Intentional difference #4 covers only a non-object *top-level* payload.
- An `overall_index` beyond `i64` loses precision (`1e+20` vs the exact digits).
- Test config files are never cleaned up.
- The **legacy** scheduler spawn (`gateway.zig:4970`) still receives no delivery budget. It is unreachable while `cron_db_path` is set, which it is on this host, but the inconsistency remains.
