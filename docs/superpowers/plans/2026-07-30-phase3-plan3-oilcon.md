# Phase ③ Plan 3 — oilcon to Rust, with storage consolidated into `price-registry`

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Port `oilcon/scripts/run.py` (345 lines) to `crates/oilcon`, replacing `lib/oil_store.py`'s own Turso database with `price-store` against `price-registry`.

**Architecture:** The riskiest of the three ports. oilcon runs **every weekday at 22:00**, it is the only one of the three with a market-data store, and this plan moves that store. Plan 0 (`price-store`, `FetchError::Upstream`) and Plans 1–2 (chipcon, inflation-con) are shipped; both of those are pure-compute ports beside their Python. This one changes where data lives.

**Tech Stack:** Rust 2021, `market-fetch` (yahoo), `price-store`, `turso-util`, `claw-core`.

**Spec:** `docs/specs/2026-07-29-con-family-rust-port-phase3-design.md` (**revision 6**). Read "Storage consolidation into `price-registry`", "Backfill: completeness, not row count", "Provenance policy" and "`chart.error`" before starting. The storage decisions were reviewed five times and several of the obvious designs are wrong for reasons recorded there — rev 5 in particular replaced the provenance-**repair** policy with a provenance **filter**, so do not implement from a cached memory of rev 4.

## Task 6 outcome — revision 8

Task 6 is **done**: **six fixture sets, all byte-identical** on message, record line and skill status — `live` (real Yahoo, lands in `rollover`), synthesised `uptrend`, `weakening-uptrend` and `no-uptrend`, and two forms of the fixed-point equality case. 78 tests in the crate, Python oracle still 28.

**The differential is genuine, which was checked before the result was believed.** Three ways it could have been tautological, all ruled out:

- `tests/differential.rs` spawns `python3 fixtures/drive_python.py` at test time and parses its stdout. The Python is executed on every run; nothing is compared against a stored expectation.
- `drive_python.py` loads the real `run.py` via `spec_from_file_location`, substitutes the three module attributes, and calls `m.build_snapshot()`, `m.format_message()` and `m.format_record_line()`. It does not reimplement the pipeline.
- The `live` fixture is real Yahoo data, not synthetic labelled as live: `symbol = CL=F`, `exchangeName = NYM`, `instrumentType = FUTURE`, 252 points spanning 2025-07-30 to 2026-07-30, closes 55.27–112.95 with every value non-integer.

**No network, established by `strace` rather than by inspection.** Zero `connect()` and zero `AF_INET` sockets in both the Python driver and the Rust differential binary. Worth recording *how* this was checked, because the obvious check fails: running the differential behind a blackhole proxy proves nothing, since `differential.rs` calls `.env_remove("http_proxy")` and `.env_remove("https_proxy")` before spawning `python3`. A probe that the code under test defeats reads exactly like a passing probe.

**One thing in the driver was removed: a reimplementation of the warning message.** The warning branch reconstructed `🛢️ OILCON 情報\n[WARN: …]\n更新：…` in Python, because `run.py` builds that string inline in `main` and there is no oracle function to call. That reconstruction would have compared one reading of the Python against another — the gap this task exists to close — and it read as coverage while proving nothing. Every set lands on `status=ok` so it was never executed; the branch now raises instead, so a mis-seeded fixture fails loudly rather than silently comparing a copy. The Rust warning message is covered by the Task 5 hand-diff against `run.py:321`, not by this differential; if the warning path ever needs differential coverage, `main` has to be driven.

**What the six sets do not cover**, stated plainly so the coverage is not overread: every intentional divergence stayed untriggered, because all sets are pre-seeded with successful fetches. The history-failure fallback, the coverage-versus-presence backfill guard, the Yahoo-only source filter, and both error-message texts are unexercised here. Each is covered by unit and contract tests instead. What the differential does establish is the thing those cannot: for a given identical input series, the message body, every number in it, the record line and the skill status agree byte for byte.

Two implementer notes, both correct. The probe skeleton handed over used a `day` column where `lib/oil_store.py` uses `date`; the column is internal to the stand-in so behaviour was unaffected, but the skeleton did not mirror the real schema as its comment claimed. And Step 1's "commit them" conflicts with the standing prohibition on `git add`/`git commit` — the fixtures were written to the working tree and left for review, which is the correct reading of the two instructions together.

## Task 5 outcome — revision 7

Task 5 is **done**: 12 contract tests, **77 in the crate**, Python oracle still 28. All nine mutations were re-applied independently and each turned its named test red. The four ported strings, the backticked job-id block, both marker lines and the history-log error line were hand-diffed against `run.py` and match verbatim. No chipcon or inflation-con message literal survives — the only occurrences are comments and the negative assertions that check those prefixes are absent.

**The inherited exit-code item was answered "assert the invariant", and the test asserting it did not.** Breaking the invariant — replacing `build_symbol_snapshot`'s WTI `return Err(...)` with a `rows: None` fallthrough — left **every contract test green**. Only Task 4's `nineteen_rows_raises_for_wti_but_none_for_brent` caught it, one layer down. The contract test's two halves were a warning-free happy path and an empty-store abort, and the empty-store case resolves inside `after_failed_refresh` long before the row-count check is reached, so neither half could see the property the test is named after.

The missing input is **WTI holding 1..20 rows with a working store**: the only way to reach `MIN_HISTORY_ROWS` with no warning already set, hence the only place where "WTI errors" versus "WTI returns `rows = None`" decides between exit 1 and a panic. Added as a third block in the same test — 19 stored rows, a failing history fetch so `after_failed_refresh` keeps them, then assert the run degrades to `[WARN: insufficient WTI history (19 rows)]` and never renders `WTI: $`. It now fails on the broken invariant by panicking at `render.rs:104`, which is the exit-101 path the item was about. **A test named after a property it does not exercise is worse than no test, because it reads as coverage.**

**One plan mutation cannot be applied as written, and the reason is the same `expect`.** "Render the full report on a warning" was specified as calling `format_message` on the warning path. A warning snapshot is the all-or-nothing empty one, so `format_message` panics on `expect("WTI rows are required")` rather than producing a full report — the mutation fails for the wrong reason, exactly the Task 3 lesson. The implementer diagnosed this itself and re-applied it as a shape mutation (injecting a `確認：` line into the warning body), which fires the named assertion for the right reason. Both forms were re-run here to confirm the diagnosis.

Three plan gaps the implementer reported, all correct:

- **The `run` signature in File Structure is a sketch.** The real surface is async, takes **two** fetchers, a `&Connection`, and **three** clock strings — `now` for deliver, `now_with_seconds` for the record line, `today` for `needs_backfill`. The one-liner `run(argv, env, fetch, store, now, out, err)` predates Tasks 3 and 4.
- **Connection failure lives in `main`, not `run`.** Python opens the DB inside `build_snapshot`; the Rust injects a `&Connection`, so credential and connect failures are handled in `main.rs` by replaying the same warning branch. Task 5 never specified that path.
- `parse_mode` was checked rather than assumed: `DeliverOptions::default()` really is `Some("Markdown")`, matching `lib/delivery.py:24`, and a test pins the value instead of trusting `..Default::default()`.

**Delivery note, not a code issue.** The delegating subagent stopped before relaying, and its monitor never reported. The work had in fact completed — the grok job log ended with a full report and a cost block — so the report was recovered by reading that job log directly, which is the sanctioned path: check the job, not the rollout.

## Task 4 outcome — revision 6

Task 4 is **done**: 17 snapshot tests, **65 in the crate**, Python oracle still 28. All eight mutations were re-applied independently and each turned its named test red. `SymbolSnapshot` and `Snapshot` moved from `render.rs` to `snapshot.rs` with a re-export; the render differential from Task 2 was **re-run and is still byte-identical** (same sha256 `912b8932…`), which is how the move was confirmed behaviour-neutral rather than by reading it.

**This plan specified the read order wrongly, and it mattered.** Revision 5 put the authoritative window read *before* `fetch_latest`. The Python's single `window()` call sits **below** the latest upsert, so the plan's order would have left today's close committed to the store but absent from `rows` — `current_close`, `today_change_pct`, both extremes, `pct_below_60d_high` and the classification all computed one observation behind, on every run. The implementer caught it, reported it as a plan-versus-file discrepancy, and followed the file. That is the seventh instruction in this phase to be wrong and the first whose consequence was every rendered number.

**It was fixed in code but not pinned by a test.** Removing the post-latest re-read left all sixteen delivered tests green — verified by applying that exact mutation. `the_latest_observation_is_present_in_the_snapshot_rows` was added here: it seeds an adequate series, returns a distinctive close on the following day from `fetch_latest`, and asserts both that the newest snapshot row carries it and that it came through the store. It turns red on the mutation. A defect that is corrected silently is one refactor away from returning.

Three further discrepancies the implementer raised were checked and are correct:

- **Short-history Brent/HO drop the stale flag.** `SymbolSnapshot(rows=None)` takes the default `stale=False`, so a symbol that both fell short *and* failed its latest fetch reports not-stale. Faithful to the file.
- **`Snapshot`'s shape is fields, not a dict**, so the all-or-nothing abort clears three `rows: None` rather than emptying a map. Same meaning.
- **252 rows cannot span 300 calendar days if they are calendar-dense.** `load_window` caps at 252 and `MIN_SPAN_DAYS` is 300, so 252 consecutive *calendar* days span 251 and would re-trigger backfill forever. The guard is safe only because Yahoo returns trading days — 252 of those run to roughly 365 calendar days, comfortably clear. **Worth recording as an assumption rather than a coincidence**: a source that ever supplied calendar-daily rows would make the guard unsatisfiable, which is the same refetch-storm shape as the provenance-repair design rev 5 removed. Test fixtures must therefore span ≥ 300 days across 252 rows, not simply be dense.

Mutation 3 was initially unobservable — every backfill-success fixture paired with a failing latest fetch, so `latest_failed` already masked the flag. The implementer added `successful_history_fetch_does_not_set_stale` rather than retuning a fixture, which is the Task 1 lesson applied without being told.

## Task 3 outcome — revision 5

Task 3 is **done**: `price_store::read_window_from_source` added with 2 new tests (price-store now 15), oilcon `store.rs` with 14 integration tests plus 1 unit test, 48 in the crate. price-cli's suite unaffected, Python oracle still 28. All eight mutations were re-applied independently and every one turned its named test red.

**The hand-rolled date arithmetic was verified exhaustively, because it had to be.** `span_days` needs a calendar difference and there is no date crate in the dependency list, so the implementation carries Hinnant's `days_from_civil`. Two things made that worth attacking rather than reading: the delivered unit test checked only three offsets, and the era line is written `if y >= 0 { y } else { y - 399 } / 400`, where whether `/ 400` binds to the whole `if` or only to the `else` arm changes the result and both parses compile. Dumped against Python's `date.toordinal()` over **every date from 1900-01-01 to 2200-12-31 — 109,938 of them, zero mismatches, strictly +1 per day**. The parse is correct and so is the algorithm. Cases that would have caught a naive implementation are included: 1900 is not a leap year, 2000 and 2400 are, and year 1 and 1600 exercise the negative-era branch.

**A user-visible error-formatting defect was fixed.** `load_window` mapped its error with `format!("{e:?}")`. The reasoning behind it was sound as far as it went — `turso_util::Error` genuinely implements neither `Display` nor `std::error::Error`, which was checked rather than assumed — but `Debug` is not the fallback to reach for. That string becomes `build_snapshot`'s warning, which renders into the delivered message as `[WARN: turso unavailable - …]` where the Python puts `str(exc)`. A Telegram reader would have received `Error { kind: Turso, message: "…" }`. `kind_str()` and `message()` are both public; the map now produces `turso: <message>`.

**One mutation had to be applied twice to be worth anything.** Dropping `AND source = ?` from the SQL while leaving three bound parameters leaves the query with two placeholders and three arguments, so libsql fails on the binding and *four* oilcon tests go red — including `load_window_empty_store_is_empty`, which cannot legitimately be sensitive to a source predicate. That red light measured a parameter-count error, not a missing filter. Re-applied with the binding reduced to match, the result is the intended one: price-store's `from_source_applies_limit_after_the_source_filter` plus exactly the three oilcon provenance tests. A mutation that fails for the wrong reason proves nothing about the test.

**The implementer's self-reported weakness is real and correctly diagnosed.** It flagged that a length assertion alone cannot catch the dropped source predicate, since a ticker holding 302 mixed rows still yields 252. Confirmed: the interleaved test survives on its date and content assertions, not its count. Mutation 5 — the Rust-side filter that reproduces the original design fault — fails it with `got 202`, which is the fault stated numerically.

Two items it raised as under-specification are accepted as such: the plan named neither `load_window`'s error type nor the `coverage` / `after_failed_refresh` function names, so both were the implementer's choice. Recorded rather than retrofitted into the plan as though they had been specified.

**A recorded limit, not fixed.** `parse_iso` validates only `1 <= m <= 12` and `1 <= d <= 31`, so `2026-02-31` parses. The comment above `is_stale` says unparseable dates are treated as stale "so a corrupt row cannot silence backfill" — true, but a *parseable impossible* date slips through, and one dated into the future would suppress the freshness condition specifically. The other two conditions still apply, and reaching this needs a corrupt row in the shared table, so day-versus-month-length validation is not being added. Written down so the guard's stated intent is not mistaken for complete.

## Task 2 outcome — revision 4

Task 2 is **done**: 17 render tests, 33 in the crate, workspace green, Python oracle still 28. All seven mutations were re-applied here and each turned its named test red; the implementer's report matched on every one, which is worth stating because the previous task's did not.

Two checks went beyond the plan and both are worth keeping in mind for Task 6.

**A render-only differential was run early, and came out byte-identical.** Four snapshots — the fixed-point equality fixture, a stale/en-dash/`n/a` combination, a 50–69-row window, and an all-negative set — were rendered by both implementations over identical inputs with the clock substituted on the Python module object, exactly the mechanism Task 6 specifies. Message, status and record line agree to the byte on all four (sha256 `912b8932…`). This does **not** discharge Task 6: it exercises no store, no fetch and no marker, which is where this port's actual risk lives. What it does buy is confidence that the string layer is not the thing that will fail there. Worth doing because the record line is assembled with Rust's `\` line-continuation, which swallows the following newline *and* its indentation — a construct that reads correctly whether or not it is correct.

**The three refusal messages were checked against `run.py` rather than accepted.** They looked like generic invented English — `"record mode requires fresh data"`, `"record mode requires non-stale data"`, `"record mode requires complete confirmation data"` — and are in fact verbatim from lines 223, 229 and 231. Suspicion was wrong, which is the point of checking rather than assuming.

**One latent divergence, recorded and deliberately not fixed here.** `format_message` calls `format_wti_line(...).expect("WTI rows are required")`. `format_wti_line` correctly returns `Result`, but `expect` converts a refusal into a panic, so this path would exit **101** where Python's uncaught `ValueError` exits **1**. It is unreachable today: `main` short-circuits on `snapshot.warning` before `format_message`, and `build_symbol_snapshot` raises for WTI rather than returning `rows = None`, so a warning-free snapshot always has WTI rows. Restructuring `format_message` to return `Result` would churn eight of the seventeen tests for a defensive gain in a path no golden can reach. **Task 5 owns exit codes and inherits this**: either propagate there, or assert the invariant that makes it unreachable. Do not leave it merely assumed.

The other three `expect`/`unwrap` sites in `render.rs` were audited and are genuinely unreachable — each sits immediately after the guard that makes it so.

## Task 1 outcome — revision 3

Task 1 is **done**: 16 tests green, the workspace green, the Python oracle still at 28. The delivered `tests/analysis.rs` is byte-identical to the block below — verified by diff, since the whole point of transcribing it literally is that a retuned constant would ship as a bug.

Two defects were found by re-running the gate rather than reading the report, and both are now fixed and pinned:

- **`compute_extremes` disagreed with Python on a tied high.** `max(enumerate(rows), key=...)` returns the **first** maximum; Rust's `Iterator::max_by` returns the **last**. `min` happens to agree — both take the first — so only the high was wrong, which is exactly the asymmetry that makes it easy to miss. `high_day` and `days_since_high` are both rendered into the message, and ties between daily closes are ordinary. Replaced with a `reduce` that keeps the first, with the `min_by` line left alone and commented so it does not get "symmetrised" later. Verified empirically against both languages, not from the documentation.
- **The lookback mutation had no coverage at the call site.** `ma_rising_uses_a_lookback_of_twenty_not_some_other_span` passes the lookback explicitly, so changing `ma_rising(rows, 50, 20)` inside `classify_oil_trend` to `10` left every test green. The same fixture separates them one level up — cur 70.5 under ma50 85.11 gives `rollover` at 20 and `no-uptrend` at 10 — so one added assertion closes it.

One mutation is recorded as **permanently unobservable, correctly**: `ma_rising`'s `n + lookback` guard is redundant with the `Err` arm beneath it, because translating Python's raise into a `Result` moved the work the guard used to do. See Step 5 item 7. No test was invented for it.

## Review record — revision 2

Reviewed by Grok 2026-07-30, then every finding corroborated against `run.py`, `lib/oil_store.py` and `price-store` before being accepted. Two things about that pass are worth carrying forward:

**Every line number in the review was fabricated.** It cited `run.py:505–527`, `:553–558`, `:627`, `:662`, `:680–688`, `:726–728`, `:765–766` and `lib/test_oilcon_run.py:1168–1175`. `run.py` is 345 lines and the test file does not reach 1168 — not one citation was in range. The review ran in plan mode against the plan text, and the numbers were dressing. **This did not make it wrong**: `cst_now` really is called inside `format_message` and `format_record_line`, `ma_rising` really returns `False` rather than raising, record mode really never delivers, and `fetch_latest` really sets stale on `None` as well as on a throw — all confirmed at their real lines. The lesson is only that a citation is not evidence, and each claim had to be re-located and re-read.

**The fix a review proposes needs the same corroboration as the finding.** Grok correctly showed the equality fixture could not falsify its mutation, then said to copy the Python's fixture instead. Running it: the Python's own fixture lands at cur 72.250 vs ma50 72.005 — strictly above, passing for an unrelated reason, with a comment describing behaviour it does not exercise. Copying it would have carried the same false green across, endorsed by a review. The fixture in Task 1 is a solved fixed point instead.

Three mutations (2, 3 and 5) were unobservable in revision 1 and their fixtures were solved numerically to make them observable; mutation 5 was not even a mutation, being algebraically identical to the original expression. Task 3's provenance policy was rewritten from repair to filter after the repair design was shown never to converge.

## Global Constraints

- **The Python stays the cron entry point** until Task 7. Do not edit `oilcon/scripts/run.py`, `lib/oil_store.py`, `lib/oil_fetch.py`, or `lib/test_oilcon_run.py`.
- **oilcon's oracle is directly runnable, unlike the other two.** Its 21 tests are `unittest.TestCase` methods, not pytest, so `python3 -m unittest discover -s lib -p 'test_oil*.py'` runs all 28 (oilcon 21, `oil_fetch` 3, `oil_store` 4) and passes today. Use it. Do not write a pytest shim, and do not claim pytest ran.
- **Where this plan's prose and `run.py` disagree, the file wins.** Report the discrepancy. Three of this phase's instructions to implementers turned out to be wrong and were caught this way.
- **Do not write to the live `price-registry` from any test.** The store seam exists so every branch is testable against an in-memory libsql.
- `cargo test` green in `~/a/claw-skills`; `cargo build --release` succeeds.
- Agents must not run `git commit`, `git add`, `git stash`, `git checkout`, `git restore`, or `cargo fmt`.
- Touch only `crates/oilcon/**` unless a step says otherwise.

### oilcon's contract differs from chipcon's and inflation-con's — do not reuse theirs

Both earlier ports share one `emit`/`main` shape. **oilcon does not.** Copying either would be the realistic accident, and every difference below is observable:

| | chipcon / inflation-con | **oilcon** |
|---|---|---|
| job id | appended unquoted | **wrapped in backticks** — `output += f"\n\n`{job_id}`"` |
| `parse_mode` | explicitly `None` | **omitted, so it defaults to `"Markdown"`** — the backticks are intentional Markdown |
| warning handling | inside the try, message rendered normally | **at the top of `main`, before mode dispatch** |
| deliver + warning | full report with a `[WARN:]` line | **a minimal three-line message only**: title, `[WARN: …]`, timestamp |
| record + warning | accepted, recorded, `degraded` | **rejected**: `[ERROR: …]` to stderr, `sys.exit(1)` |
| record status | `degraded` if warned else `ok` | **hardcoded `ok`** — a warning already exited |
| catch-all | `except Exception` around the body | **none**; `build_snapshot` has its own `try/finally` |
| history dir | `mkdir(parents=True)` first | **no mkdir** — plain `open(HISTORY_LOG, "a")` |

`format_record_line` raises on three separate conditions — a warning, any symbol `stale`, or any symbol's `rows` being `None`. All three become `[ERROR: could not write history log - …]` and exit 1.

### Test code is literal where it is written out; implementation is by contract

Task 1's tests are verbatim and every fixture was computed against the real Python first. Later tasks list what to pin and require the strings to be quoted from `run.py` — the two tasks in earlier plans where I left tests to prose are the two that needed a second pass.

---

## File Structure

```
crates/oilcon/Cargo.toml
crates/oilcon/src/lib.rs
crates/oilcon/src/analysis.rs   # moving_average, ma_rising, pct_below_60d_high,
                                # compute_extremes, compute_change_pct, classify_oil_trend
crates/oilcon/src/render.rs     # format_wti_line, format_confirmation_segment,
                                # format_message, format_record_line, fmt_price, fmt_pct
crates/oilcon/src/store.rs      # the backfill policy — the only NEW logic in this port
crates/oilcon/src/snapshot.rs   # build_symbol_snapshot, build_snapshot
crates/oilcon/src/run.rs        # run(argv, env, fetch, store, now, out, err) -> i32
crates/oilcon/src/main.rs       # thin wrapper
crates/oilcon/tests/{analysis,render,store,snapshot,contract,differential}.rs
```

---

### Task 1: the analysis layer

**Files:** Create `Cargo.toml`, `src/lib.rs`, `src/analysis.rs`, `tests/analysis.rs`. Modify the workspace `Cargo.toml`.

**Interfaces:**
- `pub struct Row { pub day: String, pub close: f64 }`
- `pub fn moving_average(rows: &[Row], n: usize) -> Result<f64, InsufficientRows>`
- `pub fn ma_rising(rows: &[Row], n: usize, lookback: usize) -> bool`
- `pub fn pct_below_60d_high(rows: &[Row]) -> f64`
- `pub fn compute_extremes(rows: &[Row]) -> Extremes` — carrying `current_day, current_close, today_change_pct, high_day, high_close, days_since_high, distance_from_high_pct, low_day, low_close, days_since_low, distance_off_low_pct`
- `pub fn compute_change_pct(rows: &[Row]) -> f64`
- `pub fn classify_oil_trend(rows: &[Row]) -> &'static str` — one of `uptrend`, `weakening-uptrend`, `rollover`, `no-uptrend`, `insufficient-history`

**Contract.** Translate line by line from `run.py`. Four things carry the behaviour and each has a test:

- **`classify_oil_trend` needs 70 rows**, and returns `insufficient-history` below that *or* if `moving_average(rows, 50)` raises. The existing Python test `test_classify_oil_trend_69_rows_insufficient_70_rows_sufficient` pins that boundary; keep it.
- **`current > ma50` is strict.** Equality is *not* an uptrend — `test_classify_oil_trend_price_exactly_equal_ma_not_uptrend` exists because that was a real bug (`f14afa4`).
- **`compute_extremes` scans the whole slice** via max/min. This is why the backfill guard in Task 3 cannot be a row count: with 70 rows the reported high/low is a 70-day extreme, not the one-year extreme the message claims.
- **`ma_rising(rows, 50, 20)`** compares `moving_average(rows, 50)` against the average of the window ending 20 rows earlier — not a shifted average of the same span.

- [ ] **Step 1: Write the failing tests**

`crates/oilcon/tests/analysis.rs`:
```rust
use oilcon::analysis::{
    classify_oil_trend, compute_change_pct, compute_extremes, ma_rising, moving_average,
    pct_below_60d_high, Row,
};

fn series(n: usize, base: f64, step: f64) -> Vec<Row> {
    (0..n).map(|i| Row { day: format!("2026-{:02}-{:02}", i / 28 + 1, i % 28 + 1), close: base + step * i as f64 }).collect()
}

fn flat(n: usize, close: f64) -> Vec<Row> {
    (0..n).map(|i| Row { day: format!("2026-{:02}-{:02}", i / 28 + 1, i % 28 + 1), close }).collect()
}

#[test]
fn moving_average_of_a_flat_series_is_that_value() {
    assert!((moving_average(&flat(50, 70.0), 50).unwrap() - 70.0).abs() < 1e-9);
}

#[test]
fn moving_average_refuses_fewer_rows_than_the_window() {
    assert!(moving_average(&flat(49, 70.0), 50).is_err());
    assert!(moving_average(&flat(50, 70.0), 50).is_ok());
}

#[test]
fn ma_rising_needs_n_plus_lookback_rows() {
    // Python's ma_rising returns False rather than raising when short.
    assert!(!ma_rising(&series(69, 60.0, 0.1), 50, 20), "69 rows is one short of 50+20");
    assert!(ma_rising(&series(70, 60.0, 0.1), 50, 20), "70 rows on a rising series must be true");
}

#[test]
fn ma_rising_compares_against_the_window_ending_lookback_rows_earlier() {
    let up = series(90, 60.0, 0.2);
    assert!(ma_rising(&up, 50, 20));
    let down: Vec<Row> = up.iter().rev().enumerate()
        .map(|(i, r)| Row { day: format!("d{i:03}"), close: r.close }).collect();
    assert!(!ma_rising(&down, 50, 20));
}

#[test]
fn ma_rising_uses_a_lookback_of_twenty_not_some_other_span() {
    // The test above pins direction, not the lookback: on a monotone series
    // every plausible lookback agrees, so it cannot see a wrong one. A series
    // that rises then rolls over separates them — 70 rows up at +0.5 to a peak
    // of 94.5, then 20 rows down at -1.2. Verified against the Python: lookback
    // 20 gives true, lookback 10 gives false.
    let mut rows = series(70, 60.0, 0.5);
    let peak = rows.last().unwrap().close;
    for j in 0..20 {
        rows.push(Row { day: format!("e{j:03}"), close: peak - 1.2 * (j + 1) as f64 });
    }
    assert!(ma_rising(&rows, 50, 20), "the 50MA is still above where it was 20 rows back");
    assert!(!ma_rising(&rows, 50, 10), "but not above where it was 10 rows back");
    // The two asserts above call ma_rising directly, so they say nothing about the
    // lookback classify_oil_trend passes. On this fixture the call site is visible:
    // cur 70.5 is under ma50 85.11, so lookback 20 gives rollover and lookback 10
    // gives no-uptrend. Without this line, `ma_rising(rows, 50, 20)` -> `10` inside
    // classify_oil_trend is caught by no test at all.
    assert_eq!(classify_oil_trend(&rows), "rollover");
}

#[test]
fn compute_extremes_reports_the_first_of_a_tied_high_not_the_last() {
    // Python takes `max(enumerate(rows), key=...)`, which is the FIRST maximum.
    // Rust's `max_by` is the LAST, so the obvious translation is wrong on ties —
    // and ties are ordinary in daily closes. `high_day` and `days_since_high` are
    // both rendered, so this is visible output, not an internal detail.
    let mut rows = flat(252, 70.0);
    rows[10].close = 120.0;
    rows[200].close = 120.0; // the tie
    rows[240].close = 40.0;
    let e = compute_extremes(&rows);
    assert_eq!(e.days_since_high, 241, "the earlier of the two highs wins");
    assert_eq!(e.high_day, rows[10].day, "and it is row 10's day that is reported");
}

#[test]
fn pct_below_60d_high_is_zero_at_the_high_and_positive_beneath_it() {
    let mut rows = flat(60, 100.0);
    assert!(pct_below_60d_high(&rows).abs() < 1e-9, "flat series sits at its own high");
    rows.last_mut().unwrap().close = 90.0;
    let p = pct_below_60d_high(&rows);
    assert!((p - 10.0).abs() < 1e-9, "10% under the 60-day high, got {p}");
}

#[test]
fn classify_needs_seventy_rows_exactly() {
    // Pinned by the Python's own boundary test, which asserts the 70 side lands
    // on `uptrend` — not merely "something other than insufficient". Verified:
    // 70 rows at +0.2 give cur 73.8 > ma50 68.9, MA rising, 0.00% off the high.
    assert_eq!(classify_oil_trend(&series(69, 60.0, 0.2)), "insufficient-history");
    assert_eq!(classify_oil_trend(&series(70, 60.0, 0.2)), "uptrend");
}

#[test]
fn price_exactly_equal_to_the_50ma_is_not_an_uptrend() {
    // `current > ma50` is strict. This was a real bug, fixed in f14afa4.
    //
    // The fixture is a solved fixed point, and it has to be. Setting the last
    // close to a previously computed MA does NOT produce equality, because the
    // last close is itself one of the 50 rows the MA averages. Python's own
    // test makes exactly that mistake: it lands on cur 72.250 vs ma50 72.005 —
    // strictly ABOVE — and passes only because pct_below is 13.99%, i.e. for
    // the wrong reason, with a comment that misdescribes it. Copying it here
    // would leave `>` -> `>=` green.
    //
    // Solve instead for x with mean(last 49 rows, x) == x, i.e. x = sum/49.
    // For 80 rows of 60.0 + 0.2i that is exactly 70.8 (mean of indices 30..78).
    let mut rows = series(80, 60.0, 0.2);
    rows.last_mut().unwrap().close = 70.8;
    let ma50 = moving_average(&rows, 50).unwrap();
    assert_eq!(rows.last().unwrap().close, ma50, "fixture must sit exactly on the MA");
    assert!(ma_rising(&rows, 50, 20), "and the MA must be rising, or the else branch is untested");
    // Strict `>` fails, MA rising -> rollover. Under `>=` this becomes `uptrend`
    // (pct_below is 6.35%), which is what makes the mutation observable.
    assert_eq!(classify_oil_trend(&rows), "rollover");
}

#[test]
fn pct_below_exactly_ten_percent_is_still_an_uptrend() {
    // `pct_below <= 10.0` is inclusive. No other fixture sits on the boundary:
    // the uptrend one is at 0.00% and the weakening one at 11.00%, so `<= 10.0`
    // -> `< 10.0` is invisible to both.
    //
    // 90 rows of 6.0 + 0.5i put the 60-day high at index 88 = 50.0; the last
    // close is forced to 45.0. (50 - 45) / 50 * 100 is exactly 10.0 in f64 —
    // most 10%-looking pairs are not (104 * 0.9 gives 9.999999999999993), so
    // this pair is chosen, not incidental.
    let mut rows = series(90, 6.0, 0.5);
    rows.last_mut().unwrap().close = 45.0;
    assert_eq!(pct_below_60d_high(&rows), 10.0, "fixture must sit exactly on the boundary");
    assert!(rows.last().unwrap().close > moving_average(&rows, 50).unwrap());
    assert!(ma_rising(&rows, 50, 20));
    assert_eq!(classify_oil_trend(&rows), "uptrend");
}

#[test]
fn a_steady_rise_within_ten_percent_of_its_high_is_an_uptrend() {
    assert_eq!(classify_oil_trend(&series(90, 60.0, 0.2)), "uptrend");
}

#[test]
fn above_a_rising_ma_but_far_off_the_high_is_a_weakening_uptrend() {
    // This fixture is fiddly and the obvious construction does not work. Pulling a
    // single close down 15% drops the price BELOW the 50MA, which is `rollover`,
    // not `weakening-uptrend` — verified against the real Python, where that
    // version gave cur 81.26 against ma50 85.51. Reaching this state needs
    // `cur > ma50` AND `pct_below_60d_high > 10` at once, so the rise has to be
    // steep enough that the average lags well behind a shallow pullback.
    //
    // Solved numerically: 90 rows rising 0.6/day, then the last 4 tapering
    // linearly to 11% below the peak. Gives cur 98.790, ma50 97.970, MA rising,
    // 11.00% below the 60-day high.
    let mut rows = series(90, 60.0, 0.6);
    let pull = 4usize;
    let peak = rows[rows.len() - 1 - pull].close;
    for j in (rows.len() - pull)..rows.len() {
        let frac = (j - (rows.len() - 1 - pull)) as f64 / pull as f64;
        rows[j].close = peak * (1.0 - 0.11 * frac);
    }
    let ma50 = moving_average(&rows, 50).unwrap();
    assert!(rows.last().unwrap().close > ma50, "fixture must stay above the 50MA");
    assert!(ma_rising(&rows, 50, 20), "fixture must keep the MA rising");
    assert!(pct_below_60d_high(&rows) > 10.0, "and must sit more than 10% off the high");
    assert_eq!(classify_oil_trend(&rows), "weakening-uptrend");
}

#[test]
fn below_the_ma_with_it_still_rising_is_a_rollover() {
    let mut rows = series(90, 60.0, 0.4);
    rows.last_mut().unwrap().close = 40.0;    // far under the average
    let ma50 = moving_average(&rows, 50).unwrap();
    assert!(rows.last().unwrap().close < ma50);
    assert!(ma_rising(&rows, 50, 20));
    assert_eq!(classify_oil_trend(&rows), "rollover");
}

#[test]
fn below_a_falling_ma_is_no_uptrend() {
    let falling: Vec<Row> = (0..90)
        .map(|i| Row { day: format!("d{i:03}"), close: 100.0 - 0.4 * i as f64 })
        .collect();
    let ma50 = moving_average(&falling, 50).unwrap();
    assert!(falling.last().unwrap().close < ma50);
    assert!(!ma_rising(&falling, 50, 20));
    assert_eq!(classify_oil_trend(&falling), "no-uptrend");
}

#[test]
fn compute_extremes_scans_the_whole_slice_not_a_suffix() {
    // The reason Task 3's backfill guard cannot be a row count: the high and low
    // are taken over every row given, so a short window silently reports a short
    // extreme while the message still says one year.
    let mut rows = flat(252, 70.0);
    rows[10].close = 120.0;    // the high is early
    rows[240].close = 40.0;    // the low is late
    let e = compute_extremes(&rows);
    assert!((e.high_close - 120.0).abs() < 1e-9, "high must come from row 10");
    assert!((e.low_close - 40.0).abs() < 1e-9, "low must come from row 240");
    assert_eq!(e.days_since_high, 241);
    assert_eq!(e.days_since_low, 11);
}

#[test]
fn compute_change_pct_is_the_last_two_closes() {
    let rows = vec![
        Row { day: "d0".into(), close: 100.0 },
        Row { day: "d1".into(), close: 110.0 },
    ];
    assert!((compute_change_pct(&rows) - 10.0).abs() < 1e-9);
}
```

- [ ] **Step 2: Run to verify it fails.**
- [ ] **Step 3: Create the crate and implement.** `mkdir -p crates/oilcon/{src,tests}` **before** adding the workspace member. Translate from `run.py`; Task 1 needs no external dependency.
- [ ] **Step 4: Run to verify it passes** — 16 tests.
- [ ] **Step 5: Mutation gate.** Apply each, confirm **at least** the named test goes red, revert. Results below are from the 2026-07-30 run, re-executed independently of the implementer's report:
  1. `classify_oil_trend` threshold 70 → 50 → **red**, `classify_needs_seventy_rows_exactly` (69 rows then reach the body and classify `rollover`, because `ma_rising` still returns false below 70).
  2. `current > ma50` → `>=` → **red**, `price_exactly_equal_to_the_50ma_is_not_an_uptrend`, which turns `rollover` into `uptrend`.
  3. `pct_below <= 10.0` → `< 10.0` → **red**, `pct_below_exactly_ten_percent_is_still_an_uptrend`, which turns `uptrend` into `weakening-uptrend`.
  4. `compute_extremes` scans only the last 70 rows → **red**, `compute_extremes_scans_the_whole_slice_not_a_suffix`.
  5. **`classify_oil_trend`'s call site `ma_rising(rows, 50, 20)` → `10`** → **red**, `ma_rising_uses_a_lookback_of_twenty_not_some_other_span`, but only because of that test's third assertion. The two direct `ma_rising` calls pass the lookback themselves, so they cannot see the call site; without the `assert_eq!(…, "rollover")` line this mutation is caught by nothing.
  6. **`compute_extremes` first-max → `max_by`** → **red**, `compute_extremes_reports_the_first_of_a_tied_high_not_the_last`.
  7. **`ma_rising`'s guard `n + lookback` → `n + 10`** → **stays green, and is genuinely unobservable.** Not a missing test: the guard is fully redundant with the `Err` arm below it. Without the guard, `moving_average(&rows[..len - lookback], n)` fails on exactly the inputs the guard rejects and returns `false` by the same path. The guard is load-bearing in Python — there `moving_average` *raises* — and becomes decoration once the raise is translated to a `Result`. Keep it for structural fidelity; do not add a test that cannot fail.

  If a mutation stays green, ask first whether it is observable at all — Plan 1 listed two that were not, both because another guard already covered the condition, and item 7 above is a third. Mutations 2, 3 and 5 were each unobservable in revision 1 and the fixtures were solved specifically to make them observable; do not simplify them back.
- [ ] **Step 6: Report**, including anything in `run.py` that contradicts this plan, and confirm `python3 -m unittest discover -s lib -p 'test_oil*.py'` still passes 28.

---

### Task 2: rendering, including the two record-mode refusals

**Files:** Create `src/render.rs`, `tests/render.rs`.

**Contract.** Translate `fmt_price`, `fmt_pct`, `format_wti_line`, `format_confirmation_segment`, `format_message` and `format_record_line`. Read them; quote every string. Points that carry behaviour:

- `format_message` sets `status = "degraded"` only when `snapshot.warning` is set. A `no-uptrend` classification with no warning is still `ok` — the trend state is a market signal, not a skill failure.
- The `OIL-TREND` line is appended **only when `rows` is present and `len >= 50`**, which is a different threshold from `classify_oil_trend`'s 70. Between 50 and 69 rows the line renders with the state `insufficient-history`.
- **The `OIL-TREND` line's own comparator disagrees with the classifier's.** `format_message` renders `above_below = "above" if current_price >= ma50 else "below"`, while `classify_oil_trend` branches on `current > ma50`. At exactly equal price the line therefore reads **`rollover (… above 50MA …)`** — "above", with a state that was reached by falling to the not-above branch. This is not a typo to clean up on the way past: it is live rendered output, so **port it as-is** and pin it with a test using Task 1's fixed-point fixture. A tidied port would be a silent behaviour change in the one place a human reads.
- `format_confirmation_segment` renders an en dash for a flat confirmation and `n/a` for a short history — both have existing Python tests (`test_flat_confirmation_renders_en_dash`, `test_confirmation_symbol_with_short_history_renders_na`). Port them.
- **`format_record_line` refuses three separate conditions**: a warning, any symbol `stale`, or any symbol's `rows` being `None`. Each raises with its own message. This is the opposite of chipcon and inflation-con, which record warned runs.
- `format_record_line` takes the timestamp as a **parameter**, as in Plans 1 and 2, purely for testability.
- **The render layer must return `Result`, not panic.** Task 1 translated `raise ValueError` as `assert!`/`panic!` in `compute_extremes`, `compute_change_pct` and `pct_below_60d_high`, which is fine there because those guards are unreachable — the `len(rows) < 20` check in `build_symbol_snapshot` and `format_record_line`'s own `rows is None` refusal mean they always receive at least 20 rows. It stops being fine one layer up: `main` wraps `format_record_line` in `except Exception` and turns a raise into `[ERROR: could not write history log - …]` with **exit 1**, whereas a Rust panic gives a panic message and **exit 101**. Both the stderr text and the exit code are part of the contract Task 5 pins, so the refusals in this layer are `Result` and the record path propagates them.

- [ ] **Step 1: Write the failing tests** — pin each of the three refusals separately (a test that only checks "warned input is refused" cannot tell them apart); the degraded/ok split; the 50-row `OIL-TREND` threshold including the 50–69 window rendering `insufficient-history`; **the `above` / `rollover` combination at exactly equal price**; the en-dash and `n/a` confirmation forms; and the exact record-line shape.
- [ ] **Step 2–4:** red, implement, green.
- [ ] **Step 5: Mutation gate.** Each of the three refusals dropped independently; `degraded` returned for `no-uptrend`; the `OIL-TREND` threshold changed to 70; **the display comparator "corrected" from `>=` to `>`**; the en dash replaced with a hyphen.
- [ ] **Step 6: Report.**

---

### Task 3: the backfill policy — the only new logic in this port

**Files:** Create `src/store.rs`, `tests/store.rs`.

**Interfaces:**
- `pub struct Coverage { pub rows: usize, pub newest: Option<String>, pub span_days: i64 }`
- `pub fn needs_backfill(cov: &Coverage, today: &str) -> bool`
- `pub async fn load_window(conn, ticker) -> Result<Vec<Row>>` — reads the newest **252** rows (`WINDOW_SIZE`, matching `run.py:265`'s `oil_store.window(conn, symbol, WINDOW_SIZE)`) **of source `yahoo` only**
- `span_days` is the **calendar** difference between the parsed oldest and newest ISO dates, not `rows - 1`. The two coincide only on a gapless daily series, which is precisely the case the guard exists to detect the absence of.
- `today` is **passed in**, never read inside. The caller supplies it from the same clock `cst_now` uses, so a run near midnight classifies staleness on the same day boundary the rendered timestamp shows. Yahoo's dates are exchange dates in a different zone; the 7-day threshold is wide enough that the mismatch cannot flip it, and tests pin `today` explicitly rather than depending on it.

**Contract — and this is where the design's four review rounds are concentrated.**

`oil_store.needs_backfill` is `SELECT 1 … LIMIT 1`: a presence check. That was safe when oilcon owned its own database. Against the shared `prices` table **one row from any writer suppresses the year-long backfill permanently**, after which oilcon only ever fetches the latest observation. WTI would stay below its thresholds indefinitely and Brent/HO would render `n/a` forever.

The replacement is three conditions, and all three are needed:

```
needs_backfill = cov.rows < 70
              || cov.newest is None or older than today - 7 days
              || cov.span_days < 300
```

- **Count alone** misses staleness and sparseness.
- **Span alone cannot reject sparse data** — two rows 365 days apart satisfy it while failing every observation threshold.
- 70 is analytic sufficiency (`classify_oil_trend`'s floor); 300 days is **horizon coverage** for the one-year extrema `compute_extremes` reports, derived from the calendar year the window represents rather than from the 20/50/70 thresholds — judgement, not calculation; 7 days spans a long weekend plus holidays.

**Provenance — filter, do not repair.** The live table already interleaves sources on one ticker: audited 2026-07-29, `SMH` carries both `stooq` and `stooq-intraday` rows. `CL=F`, `BZ=F` and `HO=F` have **no rows at all**, so oilcon's first run backfills all three. `"yahoo"` is the canonical source for these three, matching `price-cli`'s `upsert(&conn, t, &q.date, q.close, "yahoo")`, but `upsert` is public and accepts any source, so that is convention and not enforcement.

An earlier draft of this plan had a foreign-source row **trigger a repair backfill**. That is wrong twice over, and both faults are in the same direction — oilcon reaching for rows that are not its own on a table it shares:

- **It never converges.** `price_store::upsert_many` is an UPSERT. Backfilling writes Yahoo's dates; a foreign row on a date Yahoo does not return survives the repair untouched. The next run sees it again and repairs again — a backfill on **every scheduled run, forever**. Not a loop inside one process, which is why it would have passed testing; a permanent daily refetch storm against Yahoo that only shows up in production.
- **The only way to make it converge is worse.** Deleting the foreign rows first would have oilcon destroy another writer's data in a shared table to satisfy its own reader.

Foreign rows are therefore **invisible to oilcon, not a problem for it to solve**: `load_window` filters to `source = 'yahoo'`, coverage is computed over that filtered set, and `needs_backfill` fires only when *Yahoo's own* coverage is inadequate. Repair happens for the same reason as any other backfill and stops for the same reason. `price-cli` keeps writing whatever it writes.

**This filter must be in SQL, not in Rust after the read.** `price_store::read_window` applies `ORDER BY date DESC LIMIT ?` with no source predicate, so filtering afterwards yields "the yahoo subset of the newest 252 rows of any source" — which for an interleaved ticker is arbitrarily shorter than 252 and would make coverage look inadequate whenever another writer is merely active. `price-store` owns the `prices` table and oilcon must not hand-write SQL against it, so:

- [ ] **Step 0: Add `price_store::read_window_from_source(conn, ticker, source, limit)`** to `../gwebcdb/crates/price-store` — `read_window` with `AND source = ?` in the predicate. Same `limit <= 0` short-circuit, same DESC-then-reverse. Test it against a ticker seeded with two sources and assert the limit applies **after** the source filter, since that is the whole point. `read_window` stays as it is; `price-cli` is untouched.

**Stale-refresh fallback.** Freshness introduces a path that did not exist before: an established symbol that goes 8 days stale now enters history backfill, and if that request fails, the history-error path would discard stored rows that were previously usable. **A failed refresh on a symbol that already has stored history falls back to the stored rows and marks the symbol stale**, matching what a failed `fetch_latest` does today. Only an empty store may hard-fail. Because the fallback returns what `load_window` returned, it is already Yahoo-only — the filter cannot be re-entered by the failure path.

**A known limit, recorded rather than guarded.** 70 rows spread over a 300-day span satisfies all three conditions while averaging one observation per four days, and the extremes would then be computed over a thin sample while the message still says one year. No fourth density condition is added: this is the port's only logic with no Python oracle, a density rule would be invented judgement layered on invented judgement, and the shape can only arise from a partially failed write, since a Yahoo one-year fetch returns a dense ~252. Pin it with a test that asserts the current behaviour — 70 rows over 300 days does **not** backfill — so it is a decision on the record and not an oversight.

- [ ] **Step 1: Write the failing tests** — pin every boundary separately: empty; 69 vs 70 rows; fresh-but-short-span; long-span-but-stale; long-span-but-sparse (two rows a year apart); exactly 7 vs 8 days stale; **70-rows-over-300-days not backfilling** (the recorded limit); `span_days` computed from dates rather than row count, using a series with gaps; **an interleaved ticker returning only its `yahoo` rows, with a full 252 of them even though foreign rows are newer**; and the stale-refresh fallback preserving stored rows.
- [ ] **Step 2–4:** red, implement, green. Use an in-memory libsql, never the live registry.
- [ ] **Step 5: Mutation gate.** Drop each of the three conditions independently; use `>=` instead of `>` on the staleness comparison; compute `span_days` as `rows - 1`; **drop the `source` predicate from the query**; **apply the source filter in Rust after `read_window` instead of in SQL** (this one must go red on the interleaved-ticker test, and it is the mutation that reproduces the original design fault); make a failed refresh discard stored rows.
- [ ] **Step 6: Report.**

---

### Task 4: snapshot assembly

**Files:** Create `src/snapshot.rs`, `tests/snapshot.rs`.

**Contract.** Translate `build_symbol_snapshot` and `build_snapshot`, with the fetcher and the store injected. Behaviour to preserve exactly:

- **The sequence changes, because the guard changed — and the authoritative read stays last.** The Python asks `needs_backfill` *first*: it is a presence check, so it needs no data. Task 3's guard is computed from coverage, so the window must be read before the question can be asked. But the Python's single `window()` call sits **below the latest upsert**, and that placement is load-bearing. The order is:

  `load_window` (for coverage) → `coverage` → `needs_backfill(cov, today)` → optional history fetch and `upsert_many` → re-read → `fetch_latest` → optional `upsert` → **re-read** → `MIN_HISTORY_ROWS` check → `SymbolSnapshot`.

  Two reasons the window is re-read rather than assembled in memory. After a backfill, computing the working set from the fetch payload would skip whatever the upsert actually committed — the window cap, the sort order, and any pre-existing rows. And after the latest upsert, **skipping the read leaves today's close committed to the store but absent from `rows`**, so `current_close`, `today_change_pct`, both extremes, `pct_below_60d_high` and the classification would every one of them be computed an observation behind. *An earlier revision of this plan specified the read before `fetch_latest` and was wrong; it was caught during implementation. `the_latest_observation_is_present_in_the_snapshot_rows` pins it, because removing that read left the other sixteen tests green.*

- **A `fetch_history` failure resolves through `after_failed_refresh`, and this is where Tasks 3 and 4 would otherwise contradict each other.** The Python re-raises unconditionally, aborting the symbol loop and discarding symbols already built, so `build_snapshot` returns `Snapshot { symbols: {}, warning }`. Task 3 introduced a fallback for the case that guard newly created: an established symbol that goes eight days stale now *re-enters* history backfill, and a failure there must not throw away rows that were serving fine yesterday. There is only one history-fetch site in the Python, so both rules land on it. Resolve it by the state of the store, which is what `after_failed_refresh` already encodes:
  - `Ok(rows)` — the store had rows. **Keep them, mark the symbol stale, continue.** This differs from the Python, which would abort.
  - `Err(EmptyStoreOnFailedRefresh)` — nothing stored. **Abort**, as the Python does, and `build_snapshot` returns the empty all-or-nothing snapshot.

  Note the consequence and do not paper over it: a symbol holding, say, 30 rows survives a failed refresh and renders, where the Python would have lost the whole report. 30 rows clears `MIN_HISTORY_ROWS` so it is not rejected, and `classify_oil_trend` reports `insufficient-history` for it — degraded rather than absent. That is the intended trade, but it is a real divergence and Task 6's differential will show it if a history failure is exercised. Record it, and do not let the differential's report call it a bug.

  The all-or-nothing model still holds for the abort case — see the symbol-order bullet below.
- **`fetch_latest` reaches the stale flag two ways, and "failure" names only one of them.** `run.py` sets `latest_failed` in the `except` clause *and* again in the `else` of `if latest_row is not None`. A fetch that returns `None` without raising is therefore just as stale as one that throws, and nothing is written to the store in either case. A Rust port whose fetcher returns `Result<Option<Row>>` must treat `Ok(None)` and `Err(_)` identically here; only `Ok(Some(_))` clears the flag and upserts. The Python has a test for the throwing path only — `test_build_snapshot_marks_latest_fetch_failure_as_stale` — so the `Ok(None)` path needs its own test on this side. Apart from these two, nothing else sets stale.
- `len(rows) < 20` → WTI raises; Brent/HO return `rows = None`, and the message still renders with `Brent n/a`.
- Symbols are processed **WTI, Brent, HO in that order**, each writing before the next starts. A Brent failure leaves WTI's writes committed while the snapshot is discarded. Existing behaviour; preserve it, and note the coverage guard from Task 3 is what makes the leftover harmless.

**The `chart.error` mapping.** Plan 0 added `FetchError::Upstream`. Python's `parse_chart_response` returns `[]` — not an exception — for `chart.error`, a missing `result`, or falsy `closes`; Rust returns `Upstream` for the first and `NoData` for the others. **Map both to an empty vec**, exactly as chipcon does. Mapping only `Upstream` would turn an empty-history payload into a whole-snapshot failure, and this is the module where that difference is destructive: for Brent or HO it converts a local `n/a` into a total loss of the report.

- [ ] **Step 1: Write the failing tests** — pin, each separately:
  - a history failure on an **empty** store aborting the loop and discarding earlier symbols' snapshots;
  - a history failure on a **populated** store falling back, marking stale, and *not* aborting — the two halves of `after_failed_refresh`, which a single "history failure" test cannot tell apart;
  - **a throwing latest fetch and an `Ok(None)` latest fetch each setting stale, as two separate tests**, and nothing else doing so;
  - `needs_backfill` being asked *after* the window is read, and the window being re-read after a successful backfill — assert on the rows the snapshot carries, not on call order alone;
  - the WTI-raises / others-`None` asymmetry at 19 vs 20 rows;
  - symbol order, with a Brent failure leaving WTI's writes committed;
  - `Upstream` and `NoData` both becoming an empty series.
- [ ] **Step 2–4:** red, implement, green.
- [ ] **Step 5: Mutation gate.** Make the populated-store history failure abort as well (collapsing `after_failed_refresh` to always `Err`); make the empty-store failure fall back instead of aborting; set stale on a *successful* history fetch; **treat `Ok(None)` as a success that clears the flag**; compute coverage from the fetched rows instead of re-reading the window; make `len < 20` raise for Brent; reorder the symbols; map only `Upstream`.
- [ ] **Step 6: Report.**

---

### Task 5: run, markers, and oilcon's own contract

**Files:** Create `src/run.rs`, `src/main.rs`, `tests/contract.rs`.

**Contract.** `run(argv, env, fetch, store, now, out, err) -> i32`, with `main.rs` a thin wrapper. **Do not adapt chipcon's or inflation-con's goldens without changing every difference in the table at the top of this plan.** Specifically pin:

- the job id wrapped in **backticks**, and `parse_mode` left at its **default** rather than `None` — the backticks are intentional Markdown. The default is not in `run.py`: `deliver_or_fail` declares `parse_mode: str | None = "Markdown"` at `lib/delivery.py:24` and oilcon simply never overrides it, unlike the other two. Read `delivery.py`, not `run.py`, to pin this;
- **`emit_and_exit`'s order: `deliver_or_fail` first, then `emit_skill_status`, then `emit_trace`.** Not incidental — this is the ordering whose *absence* in weather caused the double-delivery fixed on 2026-07-30, and the goldens must assert the sequence, not just the presence, of the three;
- **record mode never delivers at all.** It writes the history log and then calls `emit_skill_status("ok")` and `emit_trace()` directly, bypassing `emit_and_exit`. A port that routes both modes through one exit helper would start sending Telegram messages from a mode that has never sent one;
- the warning check happening **before** mode dispatch;
- deliver + warning producing the **three-line minimal message**, not the full report;
- record + warning going to **stderr and exit 1**, not a recorded `degraded` line;
- record mode's status being a hardcoded `ok`, independent of anything in the snapshot;
- the history log opened for **append with no `mkdir`**, and a write failure producing `[ERROR: could not write history log - …]` with exit 1;
- markers emitted only when `NULLCLAW_JOB_ID` is set — `lib/trace_marker.py` makes both helpers no-ops otherwise;
- `deliver_or_fail(None, …)` echoing to stdout without calling Telegram.

- [ ] **Step 1: Write the failing goldens** from `run.py`, quoting each string.
- [ ] **Step 2–4:** red, implement, green.
- [ ] **Step 5: Mutation gate.** Unquote the job id; pass `parse_mode = None`; render the full report on a warning; accept a warned record; return 0 from the record-mode error path; emit markers with no job id; **emit the markers before delivering**; **route record mode through `emit_and_exit` so it delivers**; **use chipcon's or inflation-con's message prefix**.
- [ ] **Step 6: Report** with a grep showing no chipcon or inflation-con literal in `crates/oilcon`.

---

### Task 6: differential against the Python

**Files:** Create `crates/oilcon/fixtures/**`, `tests/differential.rs`.

Every test above asserts Rust against Rust. Only running the Python closes the gap. Both earlier ports came out byte-identical; this one has a store in the path, so the differential must hold the store constant rather than let each side build its own.

- [ ] **Step 1:** capture one Yahoo payload per symbol (`CL=F`, `BZ=F`, `HO=F`) into `fixtures/`, once, and commit them. No network in the test.
- [ ] **Step 2:** drive the Python without editing it. `build_symbol_snapshot` reaches `oil_fetch` and `oil_store` through module attributes, so a driver can replace both — `lib/test_oilcon_run.py` already patches this way with `unittest.mock`. Write `fixtures/drive_python.py` that substitutes a fixture-backed `oil_fetch` **and** an in-memory `oil_store`, then prints what is compared.
- [ ] **Step 3: substitute the Python's clock too, by the same mechanism.** Unlike Plans 1 and 2, oilcon's `cst_now()` is called **inside** the functions under comparison — `run.py:202` in `format_message` and `run.py:237` in `format_record_line` — so it cannot be injected as a parameter the way the Rust side takes `now`. It is a module-level function, so the driver replaces `oilcon_run.cst_now` with a fixed lambda, which is substitution and not editing: `run.py` stays untouched. Both `format_message`'s `更新：` line and `format_record_line`'s leading timestamp are then deterministic. Without this the comparison flakes on a minute rollover and, worse, passes most of the time.
- [ ] **Step 4: seed both sides from one fixture, not one database.** "The same store" is not achievable and the earlier wording claiming it was is wrong: the Python's `oil_store` writes an `oil_daily` table and the Rust reads `price_store`'s `prices`. They are different schemas in different files. Hold the **data** constant instead — one committed fixture of `(date, close)` triples per symbol, loaded into each side's own store by its own writer, with the row sequence asserted equal on both sides before anything is rendered. Seeding through each side's real writer keeps the store paths in the differential rather than bypassing them.
- [ ] **Step 5:** compare **the record line, the full rendered message, and the skill status**.
- [ ] **Step 6:** cover more than one trend state. A single fixture set will land in one of the four, leaving three uncompared. Synthesise the others and say which is which — Plan 2's live data landed in `WATCH` and the `RED` and boundary paths had to be added separately. Include the equality fixture from Task 1, since it is the one input where the rendered line and the classification disagree with each other.
- [ ] **Step 7:** report **every** difference including whitespace and decimal places, with no judgement about whether it matters. If byte-identical, say so and show the command.

---

### Task 7: cutover — the part that touches a live daily cron

**CUTOVER DONE 2026-07-31 12:0x.** `SKILL.md`'s `## Script` line and all three `## Usage` lines now point at `~/.nullclaw/skills/oilcon/bin/oilcon`; the binary is `sha256 18362db323145b503170ef206ae7f44eda7eb755963a9267c8eaf516d22e6f5d`, published by `tools/install-skill.sh oilcon` and matching `target/release/oilcon`. Smoke-tested through the exact path nullclaw resolves, with a job id: exit 0 in 1.3 s, full report, backticked job id, both markers, order body → status → trace, clean stderr. First scheduled Rust run is tonight at 22:00 Taipei.

**Rollback** is one line: set `## Script` back to `~/.nullclaw/skills/oilcon/scripts/run.py`. `oilcon-yanggf8`, `TURSO_DATABASE_URL` and `TURSO_AUTH_TOKEN` are untouched, so the Python resumes with its own history and its own numbers.

**⚠ A silent-failure defect was found during Step 4 and fixed before the switch.** `tools/install-skill.sh` smoke-probes a staged binary with an unknown flag and requires **exit 2** — proof that the argument parser loads and refuses. The manual `install -m 755` used earlier bypassed that gate, and running the probe by hand exposed the reason it exists: `parse_args` had `_ => {}`, so it **silently ignored unknown flags and accepted any `--mode` value**.

That is not cosmetic. `run.py` declares `choices=["deliver", "record"]`, so `argparse` exits 2 on a typo; the Rust fell through `if args.mode == "deliver"` into the **record** branch, which never delivers. Measured before the fix: `--mode recrod`, `--mode DELIVER` and `--mode ""` all exited **0** and ran as record, against **2** from the Python. A mistyped or newly-added cron flag would therefore have stopped the nightly signal while still emitting `[skill-status:ok]` — the scheduler would see success and nobody would notice the message had stopped arriving.

Fixed to match `argparse`'s three refusals — unknown flag, invalid `--mode` value, flag missing its value — all exiting 2 **before** `build_snapshot`, so a bad argument never reaches a fetch or a write. Four contract tests added and each falsified by mutation; the exit code is the contract, the message text is not byte-comparable with argparse's usage block and is not attempted. 82 tests.

The lesson is narrower than "run the installer": **a project's own install gate encodes a contract the code must meet, and bypassing it silently drops that contract.** The gate was not decoration — it caught a real defect on its first honest run.

**⚠ Found during the 2026-07-30 preflight: the port changes every derived number, permanently, and it can change the classification.** This was not on the intentional-divergence list and it is the most consequential finding of the whole plan.

The two stores hold **different closes for the same symbol on the same date**:

| date | `oilcon-yanggf8` (Python) | `price-registry` (Rust) |
|---|---|---|
| `CL=F` 2026-07-28 | 81.41999816894531 | 79.26000213623047 |
| `CL=F` 2026-07-29 | 84.72000122070312 | 84.45999908447266 |
| `HO=F` 2026-07-29 | 4.17110013961792 | 4.370100021362305 |

Not a row-count difference — a value difference, and it is systematic. Verified against the Yahoo 1-year payload captured for Task 6: its 07-28 and 07-29 closes are **79.26 and 84.46, matching `price-registry` exactly**. The Python's rows are what `fetch_latest` returned at 22:00 Taipei on each of those days, which is mid-session for the US market; the backfill's rows are Yahoo's **settled daily closes**. Both are stamped with the same date. Today's row agrees in both stores (83.69999694824219) because both took it from `fetch_latest` minutes ago — and the fixture's own 07-30 bar reads 84.25, which is the same effect visible inside one day.

Consequences:

1. **The Step 6 clause "`compute_extremes` output agrees within rounding" is unachievable and has been removed.** The inputs differ by construction, not during a transition. Measured on 2026-07-30: `ma50` 81.78 vs 82.67, WTI change −1.2% vs −0.9%, Brent −0.4% vs −1.3%, HO −1.8% vs −6.3%, distance below the 60-day high 19.4% vs 23.0%.
2. **It can flip a classification.** `classify_oil_trend` branches on `pct_below <= 10.0`. A 3.6-point gap in that figure is more than enough to put the two implementations on opposite sides of the boundary on a day that sits near it. So this is not cosmetic and not confined to display.
3. **What did agree on 2026-07-30**: the current close, both one-year extremes with their dates and day-counts, and the classification itself (`rollover`). Agreement on the extremes is partly luck — an extreme falling on a date whose two recordings differ would diverge too.
4. **The Rust series is arguably the better one.** A settled daily close is what a daily series should contain; a 22:00-Taipei snapshot is an artefact of when the job happens to run. But that is a judgement about which is preferable, not a reason to leave it undeclared.
5. **Rollback stays exact.** `oilcon-yanggf8` is untouched and keeps accumulating nothing after cutover, so reverting the `SKILL.md` line restores the old numbers as they were.

**This changes what the cutover means** — not "the same numbers from a new implementation" but "different numbers, because the historical base changes" — so it is surfaced for a decision rather than absorbed.

**Deployment is a copy followed by a proof.** On 2026-07-30 both previously ported skills were found running binaries from 07-28 while their sources had been rebuilt on 07-29. Compiling is not deploying.

**The live schedule, read from `cron.db` and cross-checked against the running daemon (2026-07-30).** These four facts change the acceptance criteria below, so they are recorded rather than assumed:

| | |
|---|---|
| job | `skill-a9d1cf6a-e8a0-4a30-8539-050188e6d33e`, `skill_name = oilcon` |
| args | `--deliver-to 7972814626` — **deliver mode only** |
| expression | `0 22 * * 1-5` with `tz_offset_s = 28800` |
| timeout | `timeout_secs = 120` |
| verification | `verification_mode = skill_contract`, `repair_policy = retry_once` |

1. **`--mode record` is not scheduled at all.** There is one job and it delivers. doughcon has two separate record jobs for comparison; oilcon has none, and there is no `~/.nullclaw/oilcon-history.log`. **Acceptance cannot include "the history line has the same shape over five scheduled runs"** — no scheduled run will ever produce one. Record mode is verified by an explicit manual invocation instead, once, and that is stated as a manual step rather than folded into the observation window.

2. **22:00 is Taipei time, and `CRON.md` says otherwise.** That file states "the cron scheduler evaluates expressions in UTC. All expressions stored in the DB are in UTC." For this job it is wrong: `last_run_secs = 1785333617` is 14:00:17 UTC and 22:00:17 Asia/Taipei, and `next_run_secs` lands on 22:00:00 Taipei exactly. The expression is read against `tz_offset_s`. Worth knowing because if the doc were right the job would fire at 06:00 Taipei and every timestamp claim in this plan would be off by eight hours. `CRON.md` lives outside both repos and is not ours to edit.

3. **The `degraded` double-delivery already happened in production.** `2026-07-20 22:00:05`: `status=error`, `verified=2`, `failure_class=contract_degraded`, `repair_action=retried_failed`. `emit_and_exit` delivers before emitting markers, so the message went out, the run was classified degraded, `retry_once` re-ran it, and it went out again. This is no longer a theoretical gap inferred from `cron.zig` — there is a record of it. The port preserves the behaviour deliberately; **a rollback trigger must not treat a degraded run as evidence the port broke**, because the Python does this too.

4. **Steady-state runs take 2–6 seconds against a 120-second timeout — but none of them backfill.** Fourteen consecutive runs measured. The Rust's first run against an empty `price-registry` must fetch a year for three tickers and write roughly 756 rows over the network, and that is the only stretch inside the 120 seconds nobody has timed. The preflight in Step 3 exists partly to remove it: once preflight has populated the store, the first *scheduled* Rust run is a steady-state run. **Preflight must be timed and the figure reported**, turning the unknown into a measurement.

- [ ] **Step 1:** `cargo build --release -p oilcon`; `sha256sum` the artifact and keep the hash.
- [ ] **Step 2:** provision the `price-registry` **write** credential and verify it out of band. `turso-util`'s cached-or-mint needs the `turso` CLI and an active login; **a cron host cannot log in interactively**, so the cache must outlive the observation window and its expiry must be detected before it bites. `price-cli doctor` reports token expiry — run it as part of acceptance.
- [ ] **Step 3: prepare and stop.** Preflight in a mode that **writes to `price-registry` and delivers nowhere**. **That mode already exists: `--mode record`.** An earlier revision said it had to be built, which was wrong — `--mode record` runs `build_snapshot` (so it backfills and upserts, writing the registry), appends the history line, emits markers, and **bypasses `emit_and_exit` entirely, so it never delivers**. Pinned by `record_mode_writes_the_history_line_and_emits_ok_without_delivering`. Two consequences, both good: **the cutover needs no new code**, which is the safest possible property for the one step that touches production; and the run doubles as the record-mode verification that Step 6 otherwise had to do by hand.

  Note what record mode does on unclean data: `format_record_line` refuses a warning, any stale symbol, or any missing rows, so it can exit 1 **after** `build_snapshot` has already written. The store is populated either way; the exit code reports whether the data came back clean. That is a useful signal, not a failure of the preflight.

  Running it writes a year of history for three tickers into the shared production registry, which is the same class of act as installing the binary, so it takes the same gate as Steps 4 and 5. **Human's call — given 2026-07-30.** Once run: **time it and report the figure** against the job's 120-second timeout (fact 4 above — this is the one stretch nobody has measured), and confirm all three tickers reach `MIN_SPAN_DAYS` of span with a newest date matching the Python's, and that the rendered message matches for the same day.
- [ ] **Step 4:** **prepare and stop.** Print the backup, `install -m 755`, and post-install `sha256sum` commands. **Human's call.**
- [ ] **Step 5:** **prepare and stop.** Print the one-line `SKILL.md` change, together with the Usage-block update — and note the ordering lesson from Plan 2: rewriting Usage to point at the binary *before* it is installed documents a path that fails. Both change with the cutover, not before.
- [ ] **Step 6:** acceptance is **parity, not `status=ok`** — 5 consecutive scheduled runs where the marker status matches the Python's and the message renders with every field populated. **The `compute_extremes`-agreement clause is removed** — the two stores hold different closes for the same dates (see the warning at the top of this task), so the figures cannot agree and their disagreement is not evidence of a defect. **The history-line clause is removed**: record mode is not scheduled (see fact 1 above), so it is verified once by explicit manual invocation and reported separately. **A `degraded` run with `repair_action=retried_failed` is not a rollback trigger** — the Python did the same on 2026-07-20 (fact 3). **"The same inputs" has to be constructed, because in production the two sides do not have them.** The Rust reads `price-registry`, freshly backfilled from a Yahoo one-year request; the Python reads `oilcon-yanggf8`, holding history accumulated daily since the skill was written. Different row sets give different extrema and can legitimately give different classifications, so a raw side-by-side of the two live runs proves nothing either way. Compare on Step 4's shared seed — export the Python store's window for the day, load it into the comparison, and run both over it — and record the live divergence separately as an observation about coverage, not as a parity failure. **Rollback triggers**: classification differing from the Python on identical inputs; a Rust-only non-zero exit; marker text, ordering or exit code differing from the goldens; stored coverage below the backfill guard after a run that reported success; or a credential that cannot be renewed. **Rollback** is reverting the `SKILL.md` line; `TURSO_DATABASE_URL`, `TURSO_AUTH_TOKEN` and the `oilcon-yanggf8` database all stay untouched throughout.
- [x] **Step 7 (done 2026-07-31):** recorded in `docs/specs/2026-07-31-phase3-oilcon-intentional-differences.md` — the intentional differences — the `Upstream`/`NoData` mapping and that a delisted symbol now surfaces only indirectly; `format_record_line` taking the clock; the coverage-based backfill replacing the presence check; the Yahoo-only read and its recorded sparse-data limit; the stale-refresh fallback; and that a `degraded` run still delivers and still trips `retry_once`, so it can deliver twice.

---

## Test Plan

| Layer | What | Gate |
|---|---|---|
| L1 | analysis — 16 tests ✅ **done** | every mutation turns at least its named test red |
| L2 | render — 17 tests ✅ **done**; three refusals pinned separately, plus the `above`/`rollover` disagreement | likewise |
| L0 | `price_store::read_window_from_source` — 2 tests ✅ **done**; limit applied after the source filter | a Rust-side filter must fail it |
| L3 | store — 14 + 1 tests ✅ **done**; every backfill boundary, Yahoo-only coverage, the recorded sparse limit | likewise; in-memory libsql only |
| L4 | snapshot — 17 tests ✅ **done**; the all-or-nothing model, both halves of `after_failed_refresh`, the latest observation reaching `rows` | likewise |
| L5 | contract — 16 tests ✅ **done**; oilcon's own markers, backticks, minimal-warning message, the inherited exit-code invariant | no chipcon or inflation-con literal survives |
| L6 | differential ✅ **done** — 6 sets byte-identical (live + 4 trend states + fixed-point equality) | every difference reported before it is accepted |
| L7 | the Python oracle still passes | `python3 -m unittest discover -s lib -p 'test_oil*.py'` → 28 |

## Acceptance Criteria

1. All Rust tests green; `python3 -m unittest discover -s lib -p 'test_oil*.py'` still passes 28.
2. Every listed mutation turns at least its named test red; results recorded, including any that could not be falsified and why.
3. The differential is byte-identical across at least two trend states, or every difference is reported and explicitly accepted.
4. `oilcon/scripts/run.py`, `lib/oil_store.py`, `lib/oil_fetch.py` and `lib/test_oilcon_run.py` are unmodified. The differential driver substitutes module attributes — including `cst_now` — which is not a modification.
5. No test writes to the live `price-registry`. `price-cli`'s own suite still passes after Step 0 adds `read_window_from_source`.
6. No chipcon or inflation-con literal appears in `crates/oilcon`.
7. The deployed binary's hash matches the built artifact's, recorded in the report.

## Out of Scope

Retiring `oilcon-yanggf8` — it is the only rollback dataset, since history is not migrated. It stays, with its token, until acceptance is met and the old and new 252-row windows have been compared for all three symbols. That retirement is a separate approved step.
