# Phase ② design — port `weather` to Rust + close the Phase ① test gaps

Status: approved design, not yet implemented
Date: 2026-07-28
Review: Claude (author) → Grok (adversarial, packet-based) → every finding re-checked against `weather/scripts/run.py` before acceptance

**Read first:** `docs/specs/2026-07-28-phase1-lessons.md`. Its headline — a fully green suite proved nothing — is why this spec spends most of its length on behaviour that a port would "fix" by accident.

## Why weather is the right second skill

Phase ① could only claim that native execution and the delivery/marker foundation work. `weather` is the full-contract gate: it is the first skill to exercise `CLAW_ENV` dotenv resolution, a primary/fallback source chain with **partial** success, unconditional `[skill-event]` on stderr, a subprocess call into the agent whose output must be sanitized, and the semantic **`failed`** status (`verified=3`) that no ported skill has emitted yet.

Live surface: 4 active cron jobs (plus 4 paused duplicates on a second Telegram account, `--account nunu` — so this is also the first real exercise of non-`main` account resolution).

## Scope

1. Port `weather` (343 lines) to Rust.
2. Grow `claw-core` by three modules: `env`, `agent`, `sanitize`.
3. **Close the three Phase ① test gaps**, which mutation testing proved real — deleting each implementation left the suite green:
   - `doughcon/src/main.rs` has no `cargo test` coverage at all;
   - the `CLAW_CONFIG` branch of `resolve_config_path` is unasserted;
   - `parse_mode` plumbing through `deliver()` is unasserted.

Out of scope: `traffic` and `mindfulness-spirit`, which also use `strip_agent_artifacts`. They come later and will reuse `claw-core::sanitize` unchanged.

## Behaviour the port must preserve

Every item below was read from `weather/scripts/run.py` (frozen) and verified. Each gets a test. **These are the things a Rust rewrite makes "better" by accident.**

### The two that change control flow

**B1 — a mid-loop CWA exception keeps partial output AND re-fetches everything.**
`format_cwa_location` sits *inside* the same `try` as the fetch:

```python
for loc in tw_locs:
    if loc in loc_map:
        line, data = format_cwa_location(loc, loc_map[loc])
        lines.append(line)          # already committed
        weather_data.append(data)
    else:
        cwa_unmatched.append(loc)
except Exception as e:
    cwa_failed_reason = f"CWA request failed with {type(e).__name__}: {e}"
if cwa_failed_reason:
    targets = tw_locs               # ALL of them, including ones already formatted
```

So if location 1 formats and location 2 raises, location 1 keeps its CWA line **and** gets an Open-Meteo line — a duplicate. A Rust `?` that aborts the branch loses line 1; falling back only for `cwa_unmatched` loses the duplicate. Neither is parity. The Rust must accumulate into the same `lines`/`weather_data` vectors and, on error, set the reason without discarding what it already pushed.

**B2 — `fallback_used` does not require the fallback to succeed, and `failed` outranks `degraded`.**

```python
fallback_used = True        # set whenever targets is non-empty, NOT gated on fb_data
...
if not weather_data:  status = "failed"
elif fallback_used:   status = "degraded"
else:                 status = "ok"
```

CWA down and every Open-Meteo call failing yields `failed` — while `emit_fallback` has already been written to stderr. Setting `fallback_used` only on success, or checking `fallback_used` first, both diverge.

### Formatting and value handling

**B3 — HKO always prints `香港`, never the requested name.** `--location 九龍` produces `🌤 香港：…` and a data row whose `location` is `香港`. Using `loc_name` is the obvious "fix" and is wrong.

**B4 — one HKO fetch, N formatted lines.** Every HK alias in the argument list produces its own line and its own `weather_data` row from the *same* fetched payload.

**B5 — three different rain formats.** HKO line: `降雨概率{psr}` with **no `%`**. TW lines: `降雨機率{pop}%`. The LLM prompt: `降雨{d['pop']}%` — and for HKO rows `pop` holds PSR, a qualitative string, so the prompt genuinely contains **`降雨高%`**. It looks like a bug; it is the contract.

**B6 — `str(int(round(x)))` uses Python's banker's rounding.** Verified: `round(24.5)==24`, `round(26.5)==26`, while Rust's `f64::round` gives `25` and `27`. Every Open-Meteo temperature ending in `.5` differs by one degree unless the Rust rounds half-to-even.

**B7 — asymmetric `weather_data` appends.** CWA appends unconditionally; HKO and Open-Meteo append only `if data:` (an empty dict is falsy). Since status keys on `weather_data` being non-empty, unifying these changes the status.

**B8 — `CWA_API_KEY=""` is treated as unset.** `if not api_key:` catches both. Checking only for a missing variable takes the wrong branch.

**B9 — `.get("location", []) or []` collapses JSON `null` to `[]`.** A present-but-null key must behave exactly like a missing one, not like a deserialization error.

**B10 — three distinct HTTP timeouts.** HKO **20s**, CWA **8s**, Open-Meteo **8s**, agent subprocess **30s**. A uniform timeout is a silent behaviour change under load.

**B11 — dotenv strips quote *characters*, not paired quotes.** `val.strip().strip('"').strip("'")` is successive character-set stripping. Verified: `"value` (unpaired) → `value`; `"'value'"` → `value`. "Remove matching surrounding quotes" is a different function.

**B12 — dotenv never overrides.** `if key not in os.environ` — a variable already set by cron wins over the file.

**B13 — the agent subprocess ignores its exit code.** `result.stdout` is used regardless of `returncode`; only an exception (including timeout) takes the `[WARN] LLM clothing advice failed: {e}` path, which prints to stderr and yields an empty advice, omitting the line.

**B14 — `"0"` is truthy.** `if pop:` / `if psr:` keep a zero probability. `if pop != 0` would drop it.

**B15 — CWA slot selection reads the wall clock.** `datetime.now(timezone(timedelta(hours=8)))` picks the nearest `startTime`. This is non-determinism *inside the code under test*, not in the LLM.

## Architecture

`claw-core` gains three modules, all shared with the later `traffic` and `mindfulness-spirit` ports:

| Module | Responsibility |
|---|---|
| `env.rs` | `$CLAW_ENV` else `~/.nullclaw/.env`; skip blank / `#` / no-`=`; strip whitespace then `"` then `'` as character sets (B11); set only when absent (B12) |
| `agent.rs` | Run `<HOME>/nullclaw/zig-out/bin/nullclaw agent -m <prompt>`, 30s timeout, take stdout regardless of exit code (B13), sanitize it, return the advice or empty |
| `sanitize.rs` | `strip_agent_artifacts` — four regex substitutions plus trim, byte-exact |

The `weather` binary crate:

```
sources/hko.rs          fetch + format, 20s
sources/cwa.rs          fetch + format + slot selection, 8s
sources/open_meteo.rs   fetch + format, 8s
routing.rs              HK-vs-TW split, argument defaults
orchestrate.rs          the fallback state machine (B1, B2) — testable without HTTP
main.rs                 thin: parse args, call orchestrate, deliver, finish
```

`orchestrate.rs` exists so the fallback state machine — the part with the two BLOCKER-class behaviours — is reachable from `cargo test` without a network. Phase ① shipped a `main.rs` that no unit test could touch; this is the correction.

**`agent.rs` resolves the binary from `HOME`**, exactly as Python's `os.path.expanduser` does. That is deliberate: it makes `HOME` a *shared* injection seam (see below), with no change to the frozen Python.

## The acceptance oracle

The advice line comes from an LLM. An earlier draft of this spec claimed the *presence* of that line was deterministic even if its text was not. **That was wrong** — `if advice:` gates on the sanitized subprocess stdout, so a timeout, an empty reply, or a reply that sanitizes to nothing all remove the line. Output *shape* depends on the LLM, not just output text.

The fix is a **shared** injection seam, not a Rust-only one:

> Both implementations resolve the agent binary through `HOME`. The differential harness already overrides `HOME` (Phase ① did it to isolate the history log). Placing a fake agent at `$STAGE/nullclaw/zig-out/bin/nullclaw` that prints a fixed string therefore redirects **both** implementations to the same stub, with zero modification to the frozen Python and no divergence in code path.

Verified: `HOME=/tmp/fakehome python3 -c "os.path.expanduser('~/nullclaw/…')"` → `/tmp/fakehome/nullclaw/…`.

With that seam the whole output becomes deterministic, and the differential compares it byte for byte, with three masks for values that are non-deterministic *by nature* rather than by implementation:

- `elapsed_ms` in the `[skill-event]` line — timing;
- exception text embedded in WARN and `cwa_failed_reason` strings — `type(e).__name__` and `str(e)` differ between runtimes and are declared intentional differences;
- nothing else.

**B15 needs its own answer**, because the wall clock is inside the code under test and cannot be injected in the frozen Python: CWA fixtures are built so that slot selection is time-invariant — every `startTime` in the fixture is far enough in the past that `best_idx` is stable regardless of when the harness runs. A fixture whose slots straddle "now" would flake, and that flake would look like a port bug.

## Test plan

**L1 — unit, Rust, written before each implementation.** `env` (precedence, the character-set quote strip, the no-override rule), `agent` (argv construction, exit-code-ignored, timeout → empty + stderr WARN), `sanitize`, `routing`, and — the point of `orchestrate.rs` — the fallback state machine including B1 (partial then raise) and B2 (fallback used but everything failed).

**L2 — sanitizer corpus differential.** Dozens of real agent outputs — closed `<ncchoices>`, **unclosed** `<ncchoices>`, button markup, blank-line runs, leading/trailing whitespace, CJK, empty — fed to both Python and Rust, compared character by character. This is the highest-risk surface: 42 lines of regex shared by three skills, existing because artifacts once leaked into Telegram. Python's `re.S`/`re.M`/`re.I` semantics, greedy-vs-lazy, substitution order, and `.strip()` vs Rust `trim()` (different character sets) all live here.

**L3 — end-to-end differential.** Three HTTP stubs, each independently settable to success / failure / partial, plus the fake agent via `HOME`. Cases must include: HK only, TW only, mixed, multiple HK aliases (B4), CWA total failure, CWA partial match, **CWA partial-then-exception (B1)**, Open-Meteo total failure (B2), empty API key (B8), `location: null` (B9), a `.5` temperature (B6), and a zero rain probability (B14).

**L4 — live cron.** Both DST-independent schedules, both Telegram accounts (`main` and the paused `nunu` jobs, unpaused for one run), a real agent call, and `last_status` unchanged from baseline.

## Closing the Phase ① gaps

- **`main.rs` coverage** — the same `orchestrate.rs` split is applied retroactively to `doughcon`: its argument parsing and branch selection move into testable functions.
- **`CLAW_CONFIG`** — a config test that sets the variable and asserts resolution, so deleting the branch turns a test red.
- **`parse_mode`** — a delivery test that inspects the request body produced through `deliver()`, so dropping it on the way to `SendOptions` fails.

Each is verified the Phase ① way: apply the mutation, confirm the intended test goes red, revert.

## Acceptance criteria

1. All four test layers pass; `cargo test` is clean **without** `--test-threads=1`.
2. Every difference between implementations appears in the intentional-differences list; the list is extended, not replaced.
3. The three Phase ① mutations (delete `CLAW_CONFIG` branch, drop `parse_mode`, invert a `main.rs` status) each now turn a test red — demonstrated, not asserted.
4. B1 and B2 are each covered by a test that fails under the "obvious" Rust rewrite.
5. After cutover, all four active weather jobs report `last_status=ok`, a real Telegram message arrives, and rollback is exercised.

## Risks

- **`strip_agent_artifacts` is shared.** A regression hits `traffic` and `mindfulness-spirit` too, which still run the Python — so the Rust and Python versions must stay behaviourally identical for as long as both exist.
- **B6 (rounding) is invisible in most fixtures.** Without a deliberate `.5` case nothing catches it.
- **The fake-agent seam depends on `HOME` resolution staying the way both implementations do it today.** If the Rust ever resolves the agent path differently — an absolute constant, an env var — the shared seam silently becomes a Rust-only one and the differential stops proving anything.
