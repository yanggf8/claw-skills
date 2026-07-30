# Phase ③ Plan 2 — inflation-con to Rust

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Port `inflation-con/scripts/run.py` (331 lines) to `crates/inflation-con`, using `market-fetch::fred` for FRED and `claw-core` for delivery/markers, with its 33 tests as the oracle.

**Architecture:** Same six-layer shape as Plan 1 (chipcon), because `emit` and `main` are structurally identical between the two skills. What differs is the classification: inflation-con's ladder is `if / elif / elif / else` — **mutually exclusive** — where chipcon's RED and ORANGE accumulate. That difference is the single most likely thing to be lost in translation.

**Tech Stack:** Rust 2021, `market-fetch` (the `fred` module), `claw-core`, `serde_json`.

**Spec:** `docs/specs/2026-07-29-con-family-rust-port-phase3-design.md` (revision 4). Read "The three skills are less alike than rev 1 and rev 2 assumed" and "inflation-con carries a reachable crash and a piece of dead code" first.

## Global Constraints

- **The Python is the oracle, and it is now correct.** The `be_last[1]` crash was fixed in `899b7a8` before this plan was written, precisely so the differential in Task 5 would mean something — a differential against an oracle that crashes only proves both sides crash. Do not port the pre-fix behaviour.
- **The Python stays the cron entry point** until Task 6. Do not edit `inflation-con/scripts/run.py`; `test_run.py` may only be read.
- **Where this plan's prose and `run.py` disagree, the file wins.** Report the discrepancy. Two claims about chipcon in an earlier design revision were simply false, so this is not a formality.
- **This environment has no working `pytest`.** The shim on `PATH` cannot import the module. The 33 Python tests were last run through a minimal stand-in implementing only `approx`, `fail`, `raises`, `skip`, `monkeypatch` and `tmp_path`. If you need to run them, say which runner you used — do not report "pytest passed" when it was not pytest.
- `cargo test` green in `~/a/claw-skills`; `cargo build --release` succeeds.
- Agents must not run `git commit`, `git add`, `git stash`, `git checkout`, `git restore`, or `cargo fmt`.
- Touch only `crates/inflation-con/**` unless a step says otherwise.

### One wart preserved, one piece of dead code dropped

1. **`load_config` runs outside `main`'s `try`** — a malformed config produces neither markers nor the controlled `INFLATION-CON failed:` line. Same as chipcon. Preserved.
2. **The `if not core_cpi_hot_3_or_6:` inside the YELLOW branch is unreachable.** YELLOW's entry condition requires that flag to be true, so its negation never holds and `"core CPI not confirming yet"` has never printed. **Do not port it.** Record it in Task 6 — carrying dead code across a translation leaves the next reader hunting for the case that triggers it.

### Test code is literal; implementation is by contract

**Test code below is verbatim and must be used as written.** Every fixture in Task 1 was run through the real Python before being written down, and the branch each lands in is recorded beside it. Two fixtures in Plan 1 were wrong in draft — one test was vacuous and one mutation could not be falsified — so these were computed, not reasoned about.

---

## File Structure

```
crates/inflation-con/Cargo.toml
crates/inflation-con/src/lib.rs
crates/inflation-con/src/analysis.rs   # annualized, latest, rising_over, classify
crates/inflation-con/src/render.rs     # format_message, record_line, fmt_pct, fmt_num
crates/inflation-con/src/config.rs     # load_config, DEFAULT_SERIES, VALID_STANCES
crates/inflation-con/src/fetch.rs      # fetch_all
crates/inflation-con/src/run.rs        # run(argv, env, fetch, now, out, err) -> i32
crates/inflation-con/src/main.rs       # thin wrapper
crates/inflation-con/tests/{analysis,render,fetch,contract}.rs
```

---

### Task 1: the analysis layer and the status ladder

**Files:** Create `Cargo.toml`, `src/lib.rs`, `src/analysis.rs`, `tests/analysis.rs`. Modify the workspace `Cargo.toml` members.

**Interfaces:**
- `pub struct Obs { pub day: String, pub value: f64 }`
- `pub fn annualized(rows: &[Obs], n: usize) -> Option<f64>`
- `pub fn latest(rows: &[Obs]) -> Option<&Obs>`
- `pub fn rising_over(rows: &[Obs], lookback: usize) -> Option<bool>`
- `pub enum Status { Ok, Watch, Yellow, Red, InsufficientData }`
- `pub struct Details { … }` with every key the Python dict has: `core_pce_day, pce3, pce6, cpi3, cpi6, breakeven, breakeven_day, breakeven_rising, policy_stance, core_pce_obs, reasons`
- `pub fn classify(series: &Series, policy_stance: &str) -> (Status, Details)` where `Series` carries `core_pce`, `core_cpi`, `breakeven_10y`

**Contract.** Translate `annualized`, `latest`, `rising_over` and `classify` from `run.py` line by line. Four things carry the behaviour:

- **`annualized` needs `len > n`** (`if len(rows) <= n: return None`), and computes `((now/then) ** (12/n) - 1) * 100`, returning `None` when `then <= 0`.
- **`rising_over` needs `len > lookback`**, and compares `rows[-1]` against `rows[-1-lookback]`.
- **The ladder is `if / elif / elif / else` — mutually exclusive.** A RED run carries no YELLOW reason; a YELLOW run is not also WATCH. chipcon accumulates across RED and ORANGE; this does not. Getting these two skills confused is the likeliest contamination in this phase.
- **`context_not_easing = (breakeven_ge or be_rising is True) and policy_stance != "easing"`.** An empty breakeven series makes `breakeven_ge` false and `be_rising` `None`, so the clause is false however hot the levels are — a missing series must never confirm RED.

The guard returns a **sparse** `Details`: only `core_pce_obs` and `reasons` are meaningful. Populate the rest with defaults, and do not let render read them on that path.

- [ ] **Step 1: Write the failing tests**

`crates/inflation-con/tests/analysis.rs`:
```rust
use inflation_con::analysis::{annualized, classify, latest, rising_over, Obs, Series, Status};

/// A monthly index compounding at `monthly_pct` per observation.
fn level(monthly_pct: f64, n: usize) -> Vec<Obs> {
    let mut v = vec![100.0_f64];
    for _ in 1..n { v.push(v[v.len() - 1] * (1.0 + monthly_pct / 100.0)); }
    v.iter().enumerate()
        .map(|(i, x)| Obs { day: format!("2026-{:02}-01", i % 12 + 1), value: *x })
        .collect()
}

fn daily(value: f64, n: usize) -> Vec<Obs> {
    (0..n).map(|i| Obs { day: format!("2026-06-{:02}", i % 28 + 1), value }).collect()
}

/// Monthly rate that annualizes to `annual` percent.
fn mo(annual: f64) -> f64 { ((1.0 + annual / 100.0f64).powf(1.0 / 12.0) - 1.0) * 100.0 }

fn series(pce: Vec<Obs>, cpi: Vec<Obs>, be: Vec<Obs>) -> Series {
    Series { core_pce: pce, core_cpi: cpi, breakeven_10y: be }
}

// Every expectation below was produced by running the real Python first.

#[test]
fn red_when_levels_cpi_and_context_all_confirm() {
    // pce3 = pce6 = 4.0%, cpi hot, breakeven 2.6 >= 2.5, stance restrictive.
    let s = series(level(mo(4.0), 12), level(mo(4.0), 12), daily(2.6, 70));
    let (st, d) = classify(&s, "restrictive");
    assert_eq!(st, Status::Red);
    assert_eq!(d.reasons.len(), 4, "{:?}", d.reasons);
}

#[test]
fn yellow_when_levels_are_above_target_but_short_of_red() {
    // pce3 = pce6 = 3.2% — over 3.0, under 3.5.
    let s = series(level(mo(3.2), 12), level(mo(3.2), 12), daily(2.2, 70));
    let (st, d) = classify(&s, "restrictive");
    assert_eq!(st, Status::Yellow);
    assert_eq!(d.reasons.len(), 2, "no boundary note when levels do not reach RED: {:?}", d.reasons);
}

#[test]
fn yellow_boundary_note_when_levels_reach_red_but_context_fails() {
    // Levels 4.0% reach RED, but breakeven 2.2 < 2.5 and stance is easing.
    let s = series(level(mo(4.0), 12), level(mo(4.0), 12), daily(2.2, 70));
    let (st, d) = classify(&s, "easing");
    assert_eq!(st, Status::Yellow);
    assert_eq!(d.reasons.len(), 3, "the boundary note is the third: {:?}", d.reasons);
    assert!(d.reasons.iter().any(|r| r.contains("context")), "{:?}", d.reasons);
}

#[test]
fn watch_when_pce_is_warm_but_not_confirmed() {
    let s = series(level(mo(2.6), 12), level(mo(2.0), 12), daily(2.2, 70));
    let (st, d) = classify(&s, "restrictive");
    assert_eq!(st, Status::Watch);
    assert_eq!(d.reasons.len(), 1, "{:?}", d.reasons);
}

#[test]
fn ok_when_nothing_confirms() {
    let s = series(level(mo(2.0), 12), level(mo(2.0), 12), daily(2.2, 70));
    let (st, d) = classify(&s, "restrictive");
    assert_eq!(st, Status::Ok);
    assert_eq!(d.reasons.len(), 1, "{:?}", d.reasons);
}

#[test]
fn insufficient_data_below_seven_core_pce_observations() {
    let s = series(level(mo(3.0), 5), level(mo(3.0), 5), daily(2.2, 70));
    let (st, d) = classify(&s, "restrictive");
    assert_eq!(st, Status::InsufficientData);
    assert_eq!(d.core_pce_obs, 5);
    assert!(d.pce3.is_none(), "the guard returns before computing anything");
}

#[test]
fn insufficient_data_when_core_cpi_is_missing_however_long_pce_is() {
    let s = series(level(mo(3.0), 24), vec![], daily(2.2, 70));
    let (st, _) = classify(&s, "restrictive");
    assert_eq!(st, Status::InsufficientData, "core CPI is required, not optional");
}

#[test]
fn the_ladder_is_mutually_exclusive() {
    // chipcon's RED and ORANGE accumulate reasons; this ladder does not. A RED
    // run must carry no YELLOW wording, and a YELLOW run no WATCH wording.
    //
    // The needle matters. RED legitimately says "core CPI confirms (>= 3.0% on
    // 3-mo or 6-mo)", so matching on ">= 3.0%" hits RED's own text and the test
    // fails against a correct implementation. Only YELLOW says "both >= 3.0%".
    // Verified against the real Python: RED has 1 line containing ">= 3.0%" and
    // 0 containing "both >= 3.0%".
    let red = series(level(mo(4.0), 12), level(mo(4.0), 12), daily(2.6, 70));
    let (st, d) = classify(&red, "restrictive");
    assert_eq!(st, Status::Red);
    assert!(d.reasons.iter().any(|r| r.contains("both >= 3.5%")), "RED's own PCE line: {:?}", d.reasons);
    assert!(!d.reasons.iter().any(|r| r.contains("both >= 3.0%")), "RED must not carry YELLOW's PCE line: {:?}", d.reasons);

    let yellow = series(level(mo(3.2), 12), level(mo(3.2), 12), daily(2.2, 70));
    let (st, d) = classify(&yellow, "restrictive");
    assert_eq!(st, Status::Yellow);
    assert!(!d.reasons.iter().any(|r| r.contains("not yet confirming")), "YELLOW must not carry WATCH reasons: {:?}", d.reasons);
    assert!(!d.reasons.iter().any(|r| r.contains("both >= 3.5%")), "YELLOW must not carry RED's PCE line: {:?}", d.reasons);
}

#[test]
fn an_empty_breakeven_cannot_confirm_red() {
    // breakeven_ge is false and be_rising is None, so context_not_easing is false
    // however hot the levels are. This is also the shape that used to crash.
    let s = series(level(mo(4.0), 12), level(mo(4.0), 12), vec![]);
    let (st, d) = classify(&s, "restrictive");
    assert_eq!(st, Status::Yellow, "a missing series must not confirm a regime");
    assert!(d.breakeven.is_none());
    assert!(d.breakeven_rising.is_none());
    assert!(d.reasons.iter().any(|r| r.contains("unavailable")), "the note must say so: {:?}", d.reasons);
}

#[test]
fn a_rising_breakeven_can_substitute_for_the_level() {
    // context_not_easing accepts EITHER breakeven >= 2.5 OR a rising trend.
    let mut be: Vec<Obs> = daily(2.0, 40);
    be.extend((0..40).map(|i| Obs { day: format!("2026-07-{:02}", i % 28 + 1), value: 2.3 }));
    let s = series(level(mo(4.0), 12), level(mo(4.0), 12), be);
    let (st, d) = classify(&s, "restrictive");
    assert_eq!(st, Status::Red, "a rising breakeven under 2.5 still satisfies the context clause");
    // Pin WHY it is RED, or an implementation that satisfied the clause through
    // breakeven_ge instead would also pass and the `or` would be untested.
    assert!(d.breakeven.unwrap() < 2.5, "the level alone must not qualify: {:?}", d.breakeven);
    assert_eq!(d.breakeven_rising, Some(true));
    assert!(d.reasons.iter().any(|r| r.contains("rising")), "the rising branch must be cited: {:?}", d.reasons);
}

#[test]
fn easing_stance_blocks_red_regardless_of_the_data() {
    let s = series(level(mo(4.0), 12), level(mo(4.0), 12), daily(2.6, 70));
    let (st, _) = classify(&s, "easing");
    assert_eq!(st, Status::Yellow, "policy_stance is the human's veto");
}

#[test]
fn a_cool_core_cpi_blocks_red_however_hot_pce_is() {
    // core_cpi_hot_3_or_6 is a required conjunct. Without this, dropping it from
    // the RED condition changes nothing any test can see.
    let s = series(level(mo(4.0), 12), level(mo(1.0), 12), daily(2.6, 70));
    let (st, _) = classify(&s, "restrictive");
    assert_ne!(st, Status::Red, "core CPI must confirm before RED");
}

#[test]
fn a_falling_pce_pace_changes_the_ok_wording() {
    // pce_falling affects only the OK reason text, never the status choice.
    // Rising path: 3-mo pace equals 6-mo, so not falling.
    let flat = series(level(mo(2.0), 12), level(mo(2.0), 12), daily(2.2, 70));
    let (st, d) = classify(&flat, "restrictive");
    assert_eq!(st, Status::Ok);
    assert!(d.reasons[0].contains("< 2.5%"), "steady path takes the threshold wording: {:?}", d.reasons);
}

#[test]
fn annualized_needs_strictly_more_than_n_observations() {
    assert!(annualized(&level(mo(3.0), 3), 3).is_none(), "len == n must be None");
    assert!(annualized(&level(mo(3.0), 4), 3).is_some());
}

#[test]
fn annualized_compounds_to_the_expected_rate() {
    let a = annualized(&level(mo(4.0), 13), 6).unwrap();
    assert!((a - 4.0).abs() < 0.01, "6-month window of a 4% path annualizes to 4%, got {a}");
}

#[test]
fn annualized_returns_none_when_the_base_is_not_positive() {
    let rows = vec![
        Obs { day: "2026-01-01".into(), value: 0.0 },
        Obs { day: "2026-02-01".into(), value: 100.0 },
    ];
    assert!(annualized(&rows, 1).is_none());
}

#[test]
fn rising_over_needs_strictly_more_than_the_lookback() {
    assert!(rising_over(&daily(2.0, 63), 63).is_none(), "len == lookback must be None");
    assert!(rising_over(&daily(2.0, 64), 63).is_some());
}

#[test]
fn rising_over_compares_the_endpoints_only() {
    let mut rows = daily(2.0, 64);
    rows.last_mut().unwrap().value = 2.5;
    assert_eq!(rising_over(&rows, 63), Some(true));
    rows.last_mut().unwrap().value = 1.5;
    assert_eq!(rising_over(&rows, 63), Some(false));
}

#[test]
fn latest_is_the_final_observation_or_none() {
    assert!(latest(&[]).is_none());
    let rows = daily(2.2, 3);
    assert_eq!(latest(&rows).unwrap().value, 2.2);
}

#[test]
fn details_carries_every_field_render_needs() {
    let s = series(level(mo(3.2), 12), level(mo(3.2), 12), daily(2.2, 70));
    let (_, d) = classify(&s, "restrictive");
    assert!(d.pce3.is_some() && d.pce6.is_some() && d.cpi3.is_some() && d.cpi6.is_some());
    assert!(d.breakeven.is_some() && d.breakeven_day.is_some());
    assert_eq!(d.policy_stance, "restrictive");
    assert_eq!(d.core_pce_obs, 12);
    assert!(!d.core_pce_day.is_empty());
}
```

- [ ] **Step 2: Run to verify it fails** — the crate does not exist.
- [ ] **Step 3: Create the crate and implement.** `mkdir -p crates/inflation-con/{src,tests}` **before** adding the workspace member; cargo hard-errors on an absent member directory. Translate from `run.py`.
- [ ] **Step 4: Run to verify it passes** — 20 tests.
- [ ] **Step 5: Mutation gate.** Apply each, confirm **at least** the named test goes red, revert:
  1. `annualized` guard `> n` → `>= n` → `annualized_needs_strictly_more_than_n_observations`.
  2. `rising_over` guard `> lookback` → `>= lookback` → `rising_over_needs_strictly_more_than_the_lookback`.
  3. Make the ladder four independent `if`s → `the_ladder_is_mutually_exclusive`.
  4. `context_not_easing` uses `and` instead of `or` between the level and the trend → `a_rising_breakeven_can_substitute_for_the_level`.
  5. Drop the `policy_stance != "easing"` term → `easing_stance_blocks_red_regardless_of_the_data`.
  6. Treat an empty breakeven as satisfying `breakeven_ge` → `an_empty_breakeven_cannot_confirm_red`.
  7. Drop the `not core_cpi` term from the guard → `insufficient_data_when_core_cpi_is_missing_however_long_pce_is`.
  8. Drop `core_cpi_hot_3_or_6` from the RED condition → `a_cool_core_cpi_blocks_red_however_hot_pce_is`.

  **Not listed, because it cannot be falsified:** removing `not core_pce` from the
  guard. An empty list already has `len < 7`, so the two terms fully overlap and no
  input can distinguish them — the same redundancy that made one of Plan 1's
  mutations useless. Recorded here so nobody adds a test that pins nothing.
  If a mutation stays green, ask first whether it is **observable at all** — two mutations in Plan 1 aimed at conditions another guard already covered and could never turn red.
- [ ] **Step 6: Report**, including anything in `run.py` that contradicts this plan.

---

### Task 2: rendering and the history line

**Files:** Create `src/render.rs`, `src/config.rs`, `tests/render.rs`.

**Interfaces:** `fmt_pct`, `fmt_num`, `format_message(status, details, cfg, warning) -> (String, SkillStatus)`, `record_line(status, details, warning, now: &str) -> String`, `pub struct Config { pub series: Vec<(String, String)>, pub policy_stance: String }`, `load_config(path) -> Config`, `DEFAULT_SERIES`, `VALID_STANCES`.

**Contract.**

- `format_message` returns `"degraded"` when a warning is present, `"ok"` otherwise — the **skill** status, not the classification. **A RED classification with no warning still reports `ok`.** RED is a market signal; reporting it as degraded would make the scheduler retry a successful run.
- `record_line` takes the timestamp as a **parameter**, as in Plan 1, purely so it can be tested.
- The `breakeven_rising` display is **three-valued**: `rising` / `flat/down` / `n/a`. An `Option<bool>` invites collapsing two of them.
- `INSUFFICIENT_DATA` **skips the indicator block** and renders `core PCE obs: N / 7 needed`
  instead — but it **still prints the manual-check block and the SIGNAL-ONLY trailer**.
  Note the two `FOMC 立場` occurrences differ: the indicator line is `FOMC 立場 (manual)：`
  with ASCII parentheses, the trailer uses fullwidth ones. Asserting on the bare prefix
  matches both and gives a false positive.
- `record_line` has two shapes: the `INSUFFICIENT_DATA` one carries `obs=`, the normal one carries `pce3=/pce6=/cpi3=/cpi6=/be=/stance=`. Both end with `warning=` and a dash when there is none.
- **`series` order follows `DEFAULT_SERIES` then any extra keys in file order — not
  "document order from the file".** This plan said the latter and it was wrong.
  `load_config` does `dict(DEFAULT_SERIES)` then `.update(file)`, and Python's `update`
  updates values **without moving existing keys**. Measured 2026-07-30: putting
  `nominal_10y` first in the file still leaves it last after the merge, while
  `zzz_extra` before `aaa_extra` in the file stays in that order afterwards.

  So `preserve_order`'s **only observable effect is on extra keys**, not on reordering
  the seven defaults. The order test must therefore use extra keys — a test that
  shuffles the known seven would pass either way and pin nothing. This needs
  `serde_json`'s `preserve_order` feature, and Plan 1 verified that without a test
  asserting on it the feature can be removed with everything still green.

- [ ] **Step 1: Write the failing tests** — pin, with the exact strings from `run.py`: the skill-status/classification split including RED-without-warning; the three-valued rising display; that `INSUFFICIENT_DATA` skips the indicators; both `record_line` shapes; the `warning=-` dash; the trailing `SIGNAL-ONLY` and `RED = 進入 review` lines; and that `load_config` preserves document order from the file.
- [ ] **Step 2–4:** red, implement, green.
- [ ] **Step 5: Mutation gate.**
  1. Return `"degraded"` for RED → the skill-status test.
  2. Collapse `n/a` into `flat/down` → the three-valued test.
  3. Render the indicator block for `INSUFFICIENT_DATA` → that test.
  4. Drop the `warning=-` dash → the record-line test.
  5. Store `series` in a `BTreeMap` → the order test.
  6. Remove `features = ["preserve_order"]` → the `load_config` order test.
- [ ] **Step 6: Report.**

---

### Task 3: the fetch layer

**Files:** Create `src/fetch.rs`, `tests/fetch.rs`.

**Interfaces:** `pub fn fetch_all(series: &[(String, String)], fetch: &dyn Fn(&str) -> Result<Vec<Obs>, CreditError>) -> Result<(BTreeMap<String, Vec<Obs>>, Option<String>), String>`

**Contract.** Translate `fetch_all`. Per series: fetch, and on error push `fetch {SERIES_ID}: {err}` with an empty vec; on an empty result push `{SERIES_ID}: no rows`. After the loop, **hard-fail only if `core_pce` is empty**, with the joined warnings or `FRED: no core PCE (PCEPILFE) — primary series`.

**Two facts measured rather than assumed, both of which shape this task:**

- **`cosd` changes nothing.** `market-fetch::fred::build_url` always sends `cosd`, where the Python omits it. Measured 2026-07-29 across all seven configured series: **identical row counts with and without it** (PCEPILFE 809, CPILFESL 834, PCEPI 809, CPIAUCSL 954, T10YIE 6149, DFII10 6148, DGS10 16845). The ~3-year default window applies to the licence-restricted ICE/BAML series used by `price cds`, not to these. So no output changes — including `core_pce_obs`, which is rendered.
- **The User-Agent must be the fixed composite, not the historical literal.** FRED now refuses a bare `nullclaw/1.0` — and refuses it by hanging the connection rather than returning 4xx, so the symptom is a timeout that reads as a network fault. `fred_fetch.py` was corrected to `curl/8.5.0 nullclaw/1.0`; matching is on the leading token. **Port the fixed literal.** "Preserve current behaviour" is only safe when the current behaviour works, and nobody had measured it until inflation-con had been broken for three weeks.

**A `NoData` mapping IS needed — an earlier draft of this plan said otherwise and was
wrong.** The reasoning that led there was that `fetch_all` already tolerates each
series, so a throwing adapter cannot crash the run. True, and irrelevant: what
changes is the **warning text**.

Measured 2026-07-30. Python's `fred_fetch.parse_csv` returns `[]` for a
header-only CSV, an all-`.` CSV, and an empty body. `market-fetch`'s
`parse_fred_csv` returns `Err(CreditError::NoData)` for all three. So the same
upstream response produces:

| | Python | Rust without a mapping |
|---|---|---|
| header-only CSV | `PCEPILFE: no rows` | `fetch PCEPILFE: no usable observations` |

That text reaches the delivered message and the history line, and Task 5's
differential compares it character for character.

The root cause is not `chart.error`, which is Yahoo-specific — it is that **the
Python parsers use an empty list to mean "no data" while the Rust ones use an
error**. That holds for both skills, which is why chipcon needed the same mapping.
Generalising from "chipcon's reason was chart.error" is what produced the wrong
instruction.

**So: map `CreditError::NoData` to an empty vec, and let `Http` and `Parse` stay
errors.** A test must pin both directions, and the mutation "map only errors
through, never NoData" must turn it red.

The one asymmetry to preserve: a failure in an unused context series
(`headline_pce`, `real_yield_10y`, `nominal_10y`) still degrades the run even though
`classify` never reads it.

- [ ] **Step 1: Write the failing tests** — pin: a successful fetch; `core_pce` empty is a hard error; `core_pce` erroring is a hard error; a secondary failure warns and continues; an empty secondary warns with the `no rows` wording; a context-series failure still degrades; and that warnings follow config order rather than alphabetical.
- [ ] **Step 2–4:** red, implement, green.
- [ ] **Step 5: Mutation gate.** Treat a secondary failure as fatal; treat an empty
  `core_pce` as non-fatal; sort the series; swap the two warning wordings; send a bare
  User-Agent; and let `NoData` through as an error rather than mapping it to empty.

  **Watch the needle width.** A test asserting `contains("PCEPILFE: no rows")` also
  matches `"fetch PCEPILFE: no rows"`, so it cannot tell the two wordings apart. The
  wording pair needs a negative assertion — this exact hole was found and reported
  during Task 3's first pass.
- [ ] **Step 6: Report**, including the exact User-Agent your fetcher sends.

---

### Task 4: run, markers, and the nullclaw contract

**Files:** Create `src/run.rs`, `src/main.rs`, `tests/contract.rs`.

**Interfaces:** `pub fn run(argv, env, fetch, now, out, err) -> i32`, with `main.rs` a thin wrapper — the same seam Plan 1 used, for the same reason.

**Contract.** `emit` and `main` are **structurally identical to chipcon's**: job id appended as `\n\n{id}` **unquoted**, `parse_mode = None`, deliver then `skill-status` then `trace`; record mode appends the history line then emits `degraded` if warned else `ok` and returns 0; the error path prints to **stderr** and returns **1**; `load_config` sits outside the try.

**What differs, and must be pinned separately rather than copied from Plan 1:**

| | chipcon | inflation-con |
|---|---|---|
| message prefix | `💾 CHIPCON 情報` | `📈 INFLATION-CON` |
| stderr prefix | `CHIPCON failed: ` | `INFLATION-CON failed: ` |
| history log | `~/.nullclaw/chipcon-history.log` | `~/.nullclaw/inflation-con-history.log` |
| statuses | 6 | 5 (`OK`/`WATCH`/`YELLOW`/`RED`/`INSUFFICIENT_DATA`) |
| trailing lines | `SIGNAL-ONLY` only | `SIGNAL-ONLY` **plus** a `RED = 進入 review` line |

Also: `emit_skill_status` and `emit_trace` are **no-ops when `NULLCLAW_JOB_ID` is unset** — read from `lib/trace_marker.py`, not assumed — so a manual run emits no markers at all.

- [ ] **Step 1: Write the failing tests** — adapt Plan 1's nine goldens to the strings above. Pin additionally that the history log is **appended, not truncated**, and that a warned record is **accepted** with `degraded`.
- [ ] **Step 2–4:** red, implement, green.
- [ ] **Step 5: Mutation gate.** Markers before delivery; `trace` before `skill-status`; a quoted job id; markers with no job id; rejecting a warned record; returning 0 on the error path; truncating the history log; **and using chipcon's message or stderr prefix** — the most likely contamination when porting the second of three similar skills.
- [ ] **Step 6: Report.**

---

### Task 5: differential against the Python

**Files:** Create `crates/inflation-con/fixtures/**`, `tests/differential.rs`.

All the tests above assert Rust against Rust. They prove the guards hold but cannot rule out both sides being consistently wrong — if the reading of the Python was off when the tests were written, tests and implementation drift together. Only running the Python itself closes that gap. Plan 1's differential found chipcon byte-identical; this one has more arithmetic to get wrong.

- [ ] **Step 1:** capture one FRED CSV per configured series into `fixtures/`, **once**, and commit them. The test must not hit the network.
- [ ] **Step 2:** drive the Python without editing it. `fetch_all` calls `fred_fetch.fetch_series` through the module object, so a driver can replace it — the pattern `test_run.py` already uses. Write `fixtures/drive_python.py` that imports `run`, substitutes the stub, and prints what is compared.
- [ ] **Step 3:** run the Rust over the same fixtures with the clock pinned to the same string, and compare **the record line, the full rendered message, and the skill status**. The message is what the cron delivers; comparing only the record line leaves the user-visible output unchecked.
- [ ] **Step 4:** report **every** difference, including whitespace and decimal places, with no judgement about whether it matters. If byte-identical, say so and show the command used.

---

### Task 6: cutover and documentation

**Deployment is a copy followed by a proof.** On 2026-07-30 both previously ported skills were found running binaries from 07-28 while their sources had been rebuilt on 07-29: compiling is not deploying, and neither a green build nor a green soak monitor can tell you which version ran.

- [ ] **Step 1:** `cargo build --release -p inflation-con`; `sha256sum` the artifact and keep the hash.
- [ ] **Step 2:** preflight — run with `--deliver-to` omitted and compare the rendered message to the Python's for the same day.
- [ ] **Step 3:** back up the current entry point, `install -m 755` the binary, then `sha256sum` the deployed file. **The two hashes must match.** **Human's call — prepare the commands and stop.**
- [ ] **Step 4:** update the `SKILL.md` Usage block **as part of Step 5's switch, not
  before it.** Note the difference from weather and doughcon: their Usage said
  `python3 <a Rust binary>`, which was simply wrong because the binary was already
  deployed. inflation-con's Usage says `python3 …/scripts/run.py`, and that is
  **correct today** — `run.py` really is a Python script. Rewriting it to
  `bin/inflation-con` before the binary is installed documents a path that does not
  exist, which is worse than the state it replaced. Verified 2026-07-30: doing it
  early makes every Usage example fail with "no such file or directory".

  So: change Usage and `## Script` together, in the same commit as the cutover, and
  verify by running the corrected first line.
- [ ] **Step 5:** switch the scheduler — one `SKILL.md` line, **human's call**.
- [ ] **Step 6:** **acceptance is parity, not `status=ok`.** inflation-con fires on the 3rd–5th of each month, so "observe several runs" means a **full month**, not a week. Accept after one scheduled run whose status matches the Python's for the same inputs and whose history line has the same shape; keep the Python entry point for a full cycle. **Rollback triggers**: status differs from the Python on identical inputs; a Rust-only non-zero exit; marker text, ordering or exit code differing from the goldens; or a changed history-line shape.
- [ ] **Step 7:** record the intentional differences and the operational notes:
  - the dropped dead code, and `format_message`'s unused `cfg` parameter — present in
    the Python signature and never read in its body;
  - `load_config` keeps unknown top-level keys in its returned dict; the Rust `Config`
    accepts only `series` and `policy_stance`. No skill behaviour depends on the extras;
  - `record_line` taking the clock as a parameter;
  - the fixed FRED User-Agent, asserted in a test rather than inherited from
    `market-fetch`'s default;
  - **`cosd` is measured inert, not provably inert.** All seven series returned
    identical row counts with and without it on 2026-07-29. That is an observation
    about FRED's current default window, not a mathematical identity — if FRED
    changes that default, `core_pce_obs` changes and it is rendered.
  - `series` order depends on `serde_json`'s `preserve_order`;
  - **a `degraded` run still delivers and still trips `retry_once`.** `cron.zig`
    retries on `verified != 1` and `degraded` is 2, so a degraded run can deliver
    twice — the same gap weather's Option A fix left open for its own degraded
    path. Not addressed here; recorded so it is not mistaken for new.

---

## Test Plan

| Layer | What | Gate |
|---|---|---|
| L1 | analysis — 20 tests | every mutation turns at least its named test red |
| L2 | render | likewise; the order pair must not be redundant |
| L3 | fetch | likewise; the User-Agent is asserted, not assumed |
| L4 | contract | markers, ordering, exit codes, record mode, and no chipcon strings |
| L5 | differential vs Python on captured fixtures | every difference reported before it is accepted |

## Acceptance Criteria

1. All tests green; `cargo test` green across `~/a/claw-skills`.
2. Every listed mutation turns at least its named test red; results recorded, including any that could not be falsified and why.
3. The differential is byte-identical, or every difference is reported and explicitly accepted.
4. `inflation-con/scripts/run.py` and `test_run.py` are unmodified.
5. The deployed binary's hash matches the built artifact's, recorded in the report.
6. No chipcon string — message prefix, stderr prefix or history path — appears in `crates/inflation-con`.

## Out of Scope

oilcon (Plan 3), fixing the `load_config`-outside-try wart, and retiring the Python.
