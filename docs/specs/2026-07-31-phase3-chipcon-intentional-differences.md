# Phase ③ Plan 1 (chipcon) — intentional differences from the Python

Every entry is a deliberate decision. Anything not listed here is a bug.
Python oracle: `chipcon/scripts/run.py` (unchanged at cutover).
Rust crate: `crates/chipcon`. Cut over 2026-07-31; first scheduled Rust run
2026-08-01 05:30 Taipei (Tue–Sat 05:30, `timeout_secs = 180`).

Acceptance is **parity with the Python on identical inputs**, not `status=ok`.

## The list is short, and that is the point

chipcon is the simplest of the three Phase ③ ports. It has **no store** —
`update_state` fetches a fresh year from Yahoo into memory on every run — and
`format_message` takes no clock. So the storage, backfill and provenance
decisions that dominate oilcon's list have no counterpart here, and the
differential covers correspondingly more of the surface.

## Argument parsing

- **`argparse`'s refusals are reproduced, including the exit code.** Unknown flag,
  invalid `--mode` value, and a flag missing its value all exit **2**, before the
  fetch.
- **The original port did not do this**, and the harm was specific to chipcon.
  It dispatches on `if args.mode == "record"`, so a mistyped mode still delivers
  — the damage was in the *ignored flag*: `--deliverto` instead of `--deliver-to`
  left `deliver_to` at `None`, so the report printed to stdout instead of Telegram
  and the 05:30 message simply never arrived, while the run exited 0 and emitted
  `[skill-status:ok]`. Measured before the fix: `--deliverto` exited 0 and
  rendered to stdout, against 2 from the Python.
- **This was a Phase ③-wide defect.** All three ports failed
  `tools/install-skill.sh`'s exit-2 smoke probe; `weather` and `doughcon`, from
  Phases ① and ②, passed it. The defect tracked the plan, not the implementer —
  the Phase ③ plans never specified argparse's refusals.
- **The message text is not byte-comparable** with `argparse`'s usage block and is
  not attempted. The exit code is the contract.

## Structural

- **`record_line` takes the clock as a parameter.** The Python calls
  `datetime.now(timezone(timedelta(hours=8))).strftime(...)` **inline**
  (`run.py:248`) — there is no `cst_now`-style helper. Injected here for
  testability and for the differential, which substitutes the `datetime` **class**
  on the module object rather than editing the file. The real class must be
  captured **once at load**: capturing it per fixture set means the second set
  grabs the fake and raises `TypeError`.
- **`classify` takes three slices** (`smh`, `qqq`, `soxx`) where the Python takes
  a `dict[str, list[list]]`. Different call shape, identical numeric path.
- **The job id is appended bare**, not in backticks — unlike oilcon, which quotes
  it. `parse_mode` is `None`, also unlike oilcon; the comment in `run.py` explains
  why (status names contain underscores).

## Fetch

- **`Upstream` and `NoData` both map to an empty series**, matching the Python's
  `parse_chart_response`. chipcon shares `lib/oil_fetch.py` with oilcon, so this
  is the same mapping decision recorded there.
- **A secondary symbol failing degrades locally**; only an empty `SMH` is a hard
  failure (`update_state` raises). Preserved.

## Operational (not fixed in this phase)

- **A `degraded` run still delivers and still trips `retry_once`, so it can
  deliver twice.** `verification_mode = skill_contract` and
  `repair_policy = retry_once` are set on this job as on the others. Preserved
  deliberately; **a degraded run is not a rollback trigger**.
- **`--mode record` is not scheduled.** There is one chipcon cron job and it
  delivers. Record mode is verified by explicit manual invocation.
- **`~/.nullclaw/chipcon-history.json` is stale** (last written 2026-06-03) and is
  not read by the current code. `update_state` builds its series from Yahoo each
  run. Left in place; not a Rust concern.

## Differential (Task 5)

- **Seven fixture sets, all byte-identical** on message, record line and skill
  status: `live` (real Yahoo `SMH`/`QQQ`/`SOXX`, classifies RED) plus one
  synthesised set for **each of the six `Status` values**. The classification
  printed per set is the **Python's own**, so the coverage is the oracle's claim
  rather than a directory label.
- **The test can fail**: two mutations of `render.rs` — a space appended to the
  message title, and the record line's `SMH={:.2}` widened to `{:.3}` — each
  turned it red with exit 101.
- **No network, with a positive control**: zero `connect()` and zero `AF_INET` in
  both the Python driver and the Rust test binary under `strace`, while a real
  HTTPS request under the same filter shows ten of each.
- **Not covered**: every fixture fetches successfully and warns nowhere, so the
  degraded text from a failed secondary fetch, a hard `update_state` failure,
  `--mode record` writing its file, and delivery/markers are untouched by the
  differential. Those remain on the contract and fetch tests.

## Live comparison, and why it cannot be byte-compared

A Python-versus-Rust run on live data differed in the last decimal. The obvious
explanation — intraday movement between the two fetches — was **wrong as stated**:
running the Rust twice gave byte-identical output, so nothing had moved in that
interval. Probing upstream settled it: three consecutive
`oil_fetch.fetch_history("SMH")` calls **inside one second** returned 539.07,
539.07 and 538.81. Yahoo's current-day bar updates live, both implementations are
equally subject to it, and the Python differs from itself across two runs the same
way.

So a live side-by-side proves nothing either direction, and that is the argument
for the differential using committed fixtures: **the input itself changes between
two reads.** Structural output — status, reason list, day counts — was identical
across all four runs.

## Cutover and rollback

- Binary published by `tools/install-skill.sh chipcon`, `sha256`
  `ed64a101c59a26272dcf24ffd74e760817c861ebfdf9dfaebc5e4397c011b8a1`.
- **Rollback**: set `## Script` back to
  `~/.nullclaw/skills/chipcon/scripts/run.py`. chipcon holds no persistent state
  that the port could have corrupted, so rollback is complete and immediate —
  the simplest of the three.
- **Rollback triggers**: a Rust-only non-zero exit; marker text, ordering or exit
  code differing from the goldens. **Not** a rollback trigger: a `degraded` run,
  or last-decimal differences from the Python on live data.
