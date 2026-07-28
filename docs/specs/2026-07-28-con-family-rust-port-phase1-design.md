# claw-skills Rust port — Phase ① design (foundation + doughcon pilot)

Status: approved design, not yet implemented
Date: 2026-07-28
Reviewers: Claude (author) → Codex (adversarial review, corroborated line-by-line against source)

## Why

The standing instruction is that the whole stack is Rust. `claw-skills` is currently ~8,400 lines of production Python across 12 skills plus a shared `lib/`. All 38 live nullclaw cron jobs run `verification_mode = skill_contract` — there is no lax mode left, so a skill that gets the markers wrong alerts the same day (`CLAUDE.md`, Scheduler contract). Of those 38, four (`cct`) and two (`ainews`) belong to repos outside this one; 32 are in scope across the four phases. This document covers **Phase ① only**.

## Decomposition

The full port is too large for one spec. Four phases, each with its own spec:

| Phase | Scope | Production LOC | Live jobs |
|---|---|---:|---:|
| **① (this doc)** | `claw-core` foundation + **doughcon** pilot | ~490 | 4 |
| ② | `weather` as the full-contract compatibility gate | 343 | 8 |
| ③ | Remaining con family — chipcon, oilcon, inflation-con | ~1,040 | 3 |
| ④ | Remaining daily skills, then `news` last (3,534 lines, LLM + images) | ~5,100 | ~22 |

LOC counts the skill plus the `lib/` portion ported in that phase (Phase ① is doughcon 163 + delivery/telegram/trace_marker 322). The remaining ~1,400 lines of `lib/` — `skill_runner`, `heartbeat`, `news_quality`, `cover_image`, `oil_*` — are pulled in by whichever phase first needs them; no phase ports `lib/` speculatively.

Out of scope entirely: `cct` (owned by `~/a/cct`), `ainews` (owned by `~/b/ainews`), `agent-reach` (SKILL.md only, no script).

## Verified runtime facts

**The canonical statement of the scheduler contract is `CLAUDE.md` → "Scheduler contract (hard constraints)"** (committed `c7ccf8c`, ahead of this port). It already pins the literal stdout markers, the full classification table, and the `lib/` coupling a port would break. This section is *not* a second source of truth — it records what was re-verified against nullclaw's source while designing the port, plus the facts that commit does not cover (V1, V2, V5, V7, V9, V10, V11). Where the two overlap they agree; if they ever diverge, `CLAUDE.md` wins and this file is wrong.

**V1. nullclaw executes native binaries as skills; no nullclaw change is needed to launch Rust.**
`resolveInterpreterPrefix` (`nullclaw/src/cron.zig:1928`) returns an empty prefix when frontmatter declares `interpreter: none|native`, or when the script path does not end in `.py`. `buildSkillCommand` (`:1943`) then runs the path directly.

**V2. Resolution never checks that the path exists or is executable.** `resolveSkillExecFrom` (`:1966`) only string-builds a command, which the gateway hands to the system shell. "Native binary" therefore also means: file present at fire time, executable bit set, no shell-breaking characters in the path.

**V3. Missing markers fail loudly — they do not silently pass.** `classifySkillRun` (`:7930`):

| Condition | `verified` | `failure_class` |
|---|---:|---|
| timed out | 3 | `timeout` |
| exit code ≠ 0 | 3 | `exec_error` |
| no `[trace:<id>]` | 2 | `content_invalid` |
| trace but no status | 2 | `contract_missing` |
| `[skill-status:degraded]` | 2 | `contract_degraded` |
| `[skill-status:failed]` | 3 | `contract_failed` |
| `[skill-status:ok]` + trace | 1 | — |

`gateway.zig:4662` sets `skill_status = if (verified == 1) "ok" else "error"`; `gateway.zig:4605` retries once when `verified != 1 and repair_policy == retry_once`; `gateway.zig:4668` alerts the operator on any `verified != 1`.

**V4. `degraded` is not a soft warning.** It is `verified = 2` → `last_status = error` → retry → alert. The comment in `doughcon/scripts/run.py:117-119` claiming otherwise is **wrong about the system it describes**. The port preserves the *behaviour*; it must not implement the comment's stated intent.

**V5. The trace marker must match the run's trace id exactly.** `isTraceMarkerLine` (`cron.zig:7994`) compares the marker payload to `trace_id` with `mem.eql`. `NULLCLAW_JOB_ID` is set to the per-run trace id (`cron.zig:2135`), not a stable job id — so the value must be read fresh from the environment each run.

**V6. Marker order is irrelevant and the last status wins.** `parseSkillContractMarker` (`:7977`) scans all lines, trims surrounding whitespace, and overwrites `marker.status` on each match.

**V7. Scheduled runs receive no delivery budget.** `NULLCLAW_SKILL_TIMEOUT` is set only by `buildManualSkillChildEnv` (`cron.zig:2165`); the scheduled path uses `buildCronChildEnv` (`:2155`). `cron.zig:6491` documents it literally as `NULLCLAW_SKILL_TIMEOUT=<manual skill runs only>`. **`NULLCLAW_SKILL_STARTED` does not appear anywhere in the nullclaw source.** Consequence: the elapsed-time branch of `delivery.py:_resolve_delivery_deadline` has never executed in production, and scheduled deliveries always fall back to telegram's 30 s `DEFAULT_DEADLINE_S`.

**V8. The Python `lib/` has consumers outside this repository and cannot be retired.** `CLAUDE.md:213-214`: `cct` (`~/a/cct/skills/cct`) and `autocli` (`~/.nullclaw/skills/autocli`, a real directory, not a symlink) both import `delivery` and `trace_marker` through `_resolve_skills_lib()`.

**V9. `deploy.sh` only does symlink bookkeeping and never builds.** It manages immediate directories containing a `SKILL.md` (`deploy.sh:199`), so a `crates/` directory is ignored. Its own header (`:10-24`) states it is not a pre-cron gate.

**V10. `.gitignore` does not cover skill `bin/` directories.** Verified against the current file.

**V11. Uncertain — openclaw/nanoclaw.** `CLAUDE.md:134-136` says they ignore the literal `## Script` path and discover the script relative to `SKILL.md`. The openclaw/nanoclaw implementations are not available locally, so "hardcodes `run.py`" is **not independently corroborated**. It does not bind today: `~/clawd/skills` contains only the `lib` symlink. It remains a portability constraint — changing only nullclaw's `## Script` line would leave other hosts on Python.

## Decisions

**D1. The Rust workspace lives in `~/a/claw-skills/crates/`.** Skill definition and implementation stay in one repo, one commit can change both, and `deploy.sh` is unaffected (V9).

**D2. `claw-core` library crate + one binary crate per skill.** This deliberately departs from the gwebcdb precedent (`bridge-core` + a single `bridge-cli` package declaring 8 `[[bin]]`s). Cargo has no per-`[[bin]]` dependency tables — dependency and feature selection is package-wide — and skills are heterogeneous: `news` will need LLM and image dependencies that a 163-line skill should not carry through its build. gwebcdb's bins are one cohesive CLI over the same small dependency surface, which is a different situation. **Constraint: `claw-core` must stay genuinely foundational.** If skill-specific HTTP/LLM/image dependencies leak into it, the split stops paying for itself.

**D3. Python `lib/` is frozen, not retired** (V8). Phase ① adds a Rust foundation beside it. Both implementations exist until every consumer — including the two outside this repo — has moved.

**D4. Binary at `<skill>/bin/<name>` (gitignored); `## Script` points at `~/.nullclaw/skills/<name>/bin/<name>`.** Cutover is one line in SKILL.md; rollback is reverting that line. `scripts/run.py` stays in place for the whole transition.

**D5. Tests first.** Port the existing Python tests (`test_delivery.py` 131, `test_telegram_retry.py` 154, `test_trace_marker.py` 69 lines) into Rust **before** writing the implementations. doughcon has zero tests, so characterization tests against current Python behaviour are written first.

**D6. Acceptance = differential fixtures + one full DST cycle.** Detailed below.

**D7. `jiff` for the `--et-hour` DST gate, with `tzdb-bundle-always`.** Relying on the host `/usr/share/zoneinfo` would undercut the standalone-binary premise. Python's fail-open behaviour on tz-load failure is preserved (warn on stderr, run unconditionally).

**D8. JSON is parsed at the adapter boundary into Rust types; JSON is never the app data model.** This requires an explicit intentional-differences list (below) because it changes malformed-input behaviour.

**D9. nullclaw is patched to supply the scheduled delivery budget** — `buildCronChildEnv` gains `NULLCLAW_SKILL_TIMEOUT` and `NULLCLAW_SKILL_STARTED`. This is a **behaviour change to existing Python skills**, not merely a fix: it activates a code path that has never run. For jobs with a large timeout the delivery budget widens; for a job whose own work consumes most of a short timeout, delivery will now abandon retries earlier than today. It must be validated as a change, and it must land and soak **before** the Rust cutover so the two changes are never diagnosed together.

## Architecture

```
~/a/claw-skills/
├── Cargo.toml                     # workspace
├── crates/
│   ├── claw-core/
│   │   ├── config.rs              # explicit path → $CLAW_CONFIG → ~/.nullclaw/config.json
│   │   ├── telegram.rs            # send with bounded retry
│   │   ├── delivery.rs            # deliver_or_fail behaviour matrix
│   │   ├── marker.rs              # [skill-status:] / [trace:] / [skill-event]
│   │   ├── budget.rs              # delivery deadline from the cron env
│   │   └── outcome.rs             # see "Outcome model"
│   └── doughcon/                  # pilot binary crate
├── doughcon/
│   ├── SKILL.md                   # ## Script flipped at cutover
│   ├── bin/doughcon               # build artifact (gitignored)
│   └── scripts/run.py             # retained — rollback path
└── lib/                           # frozen Python (V8)
```

Each unit answers exactly one question: `config` — what is the token; `telegram` — did the send succeed; `delivery` — how do stdout/stderr/exit split on failure; `marker` — emit the two lines; `budget` — how long may delivery take; `outcome` — what does the process exit with. None reaches into another's internals. This mirrors the existing Python split, which is already correct.

### Outcome model

Exit code and semantic status are **independent** and must not be collapsed into a `Result<()>` or a boolean:

- a timeout or non-zero exit overrides all markers (V3);
- `degraded` is exit 0 but `last_status = error` (V4);
- semantic `failed` is exit 0 with `verified = 3`;
- delivery failure prints the body to **stdout**, the diagnostic to **stderr**, then exits **before** markers;
- a DST-gate skip is exit 0 **with** markers and **no** body.

`outcome.rs` models delivery result, skill status, marker eligibility, and process exit as four separate values. The binary owns the final exit.

## Behaviour contracts to preserve

These are deliberate and easy to "clean up" by accident. Each gets a test.

**delivery**
- Both `None` and empty string mean stdout mode: print body with trailing newline, return success.
- A successful Telegram send emits **no** body.
- On failure: body to stdout **first**, then diagnostic to stderr, then `exit(1)` by default; with the opt-out, return false instead.
- Deadline parsing is forgiving: missing / malformed / non-positive timeout means "use telegram's default", not failure. A malformed start value falls back to `timeout - 1`. A future start time clamps elapsed to zero.

**telegram**
- Config precedence: explicit argument → `$CLAW_CONFIG` → `~/.nullclaw/config.json`. Any file or parse error means "no token", not a crash.
- nullclaw account lookup first; falls back to openclaw `botToken` **even in a mixed-schema file** when the account is absent.
- A missing token returns false with **no** telegram diagnostic — only `deliver_or_fail` prints one.
- Retryable: 429, 500–599, `URLError`, socket timeout, `TimeoutError`. Everything else stops immediately — **including 408**, which is currently permanent.
- The wall-clock budget starts **after** config lookup and payload construction, not at process start.
- Backoff sleeps `min(backoff, remaining)` and may consume the entire remaining budget.
- HTTP 200 is success **without parsing the response body**. `test_telegram_retry.py:55` explicitly treats a body-less fake 200 as success.
- The payload always sets `disable_web_page_preview=true`; `parse_mode` defaults to legacy `"Markdown"` and must be **omitted entirely** when null.

**markers**
- Status validation happens **before** the environment check: an invalid status must raise even during a manual run where nothing would be emitted (`test_trace_marker.py:42`).
- `emit_fallback` is never job-id gated, defaults to stderr, and has exact punctuation differences depending on whether `elapsed_ms` is supplied.

**doughcon**
- tz-load failure → warn on stderr, run unconditionally (fail-open).
- Gate mismatch → exit 0, emit `ok` + trace, no body.
- `--et-hour` is **not** range-validated; `-1` and `99` are accepted and become permanent skips.
- Upstream failure is asymmetric: deliver mode uses the delivery opt-out, ignores the returned false, emits `degraded`, exits 0 **even if both upstream and Telegram failed**; record mode prints to stderr and exits 1 **without markers**.
- A healthy-report Telegram failure exits 1 *before* markers → `exec_error`, a different class from the degraded path.
- "No data" is not `overall_index == 0`: zero becomes `-1` only when every place has null popularity, and an empty place list counts as all-null.
- Record mode emits `ok` after a successful append even when the index is `-1` — it measures capture, not data quality.
- Timestamp parsing is permissive: other ISO offsets are accepted, any error falls back silently, and a naive timestamp is interpreted in the host's local zone. The fallback string has **seconds**; the normal path is formatted to **minutes**.

### Intentional differences (D8 consequences)

Typed deserialization changes malformed-input behaviour and this is accepted, not hidden:

1. A malformed PizzINT payload (top-level array, non-object place, wrong field type) currently throws *outside* the fetch handler and becomes a hard exit. Rust will reject it at the adapter boundary and route it to the same degraded path as a fetch failure.
2. Telegram response bodies remain unparsed — HTTP 200 is success. Rust must **not** start requiring `{"ok":true}`.

Any further difference discovered during implementation is added here, never absorbed into a test that claims parity.

## Build / install / activate

`deploy.sh` never builds (V9) and resolution never checks existence (V2), so a fresh clone with a committed SKILL.md pointing at a gitignored binary produces `exec_error` — twice, because of `retry_once`. Phase ① adds a separate, strict, atomic installer:

1. build with the committed lockfile;
2. verify the artifact exists and is executable;
3. stage it under a temporary name inside `<skill>/bin/`;
4. atomically rename into place;
5. re-resolve the command the way nullclaw would and confirm it runs;
6. only then flip `## Script`.

It **refuses activation** when the binary is missing or non-executable, and it exits non-zero on failure — unlike `deploy.sh`, which always exits 0 (`:224`) and does not check `ln -s` results (`:179`). `.gitignore` gains skill `bin/` directories (V10). `deploy.sh` itself is left alone in this phase.

## Testing

**Layer 1 — ported unit tests.** The three Python test files become Rust tests before any implementation exists. Gaps the Python tests do not cover, to be added: config-schema cases, elapsed-start budgeting, fallback-event formatting, unexpected 2xx bodies, socket timeout, and backoff scheduling.

**Layer 2 — differential fixtures.** Two translations can preserve the same misunderstanding, so the oracle is the running Python, not the ported tests. For each fixture, run both implementations under identical env and compare:

- exit code
- exact stdout
- exact stderr
- Telegram request body, attempt count, per-attempt timeout, and backoff timing (against a local stub server)
- history-log append / no-append
- the `verified` / `failure_class` nullclaw would assign

Fixtures must include a synthetic `NULLCLAW_JOB_ID` — without it markers are not exercised at all — and must cover delivery success, delivery failure, missing token, upstream failure in both modes, gate skip, gate pass, all-null data, and a malformed payload.

**Layer 3 — live cron.** One "cycle" is ambiguous because the paired firings are designed so exactly one skips. Validate **both** candidate firings, **both** the delivery job and the record job, the unique trace id, the history append, the nullclaw classification, and actual Telegram receipt.

## Acceptance criteria

1. Layer 1 and Layer 2 pass with zero unexplained differences; every difference appears in the intentional-differences list.
2. The nullclaw env patch (D9) has landed with the Python skills still active, and **every affected live job has fired at least once** under it with no change in `last_status`. The slowest gate is `inflation-con` (`0 6 3-5 * *`), which fires only on the 3rd–5th of a month; if waiting for its natural firing is impractical, a manual `nullclaw cron run` of that job counts, provided the run goes through the scheduled env path and not the manual-skill path — otherwise it does not exercise the change at all.
3. The installer refuses to activate against a missing or non-executable binary.
4. After cutover, both doughcon firings and both job kinds record `last_status=ok`, a Telegram message actually arrives, and the history log gains exactly one line.
5. Rollback verified by reverting the `## Script` line and observing the Python path run clean.

Phase ① may claim only: **native execution plus the delivery/marker foundation are validated.** It may **not** claim the shared contract is complete — doughcon does not exercise `CLAW_ENV` dotenv resolution, partial primary/fallback results, `[skill-event]`, semantic `failed`, or the nullclaw subprocess call. `weather` does, which is why it is Phase ②.

## Risks

- **D9 changes live Python behaviour.** Mitigated by landing it separately and soaking before cutover.
- **`claw-core` scope creep.** If it accumulates skill-specific dependencies, D2's rationale evaporates. Reviewed at each phase boundary.
- **V11 is uncertain.** If openclaw or nanoclaw is ever activated for these skills, a `## Script` flip is not sufficient. Re-verify before that happens.
- **Two implementations of delivery coexist** for the duration of the port (D3). A behaviour fix must be applied to both until the Python side is retired.
