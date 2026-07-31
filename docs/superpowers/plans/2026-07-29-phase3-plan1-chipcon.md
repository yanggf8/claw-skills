# Phase ③ Plan 1 — chipcon to Rust

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Port `chipcon/scripts/run.py` (302 lines) to `crates/chipcon`, using `market-fetch` for Yahoo and `claw-core` for config/delivery/markers, with its 11 existing tests as the oracle.

**Architecture:** chipcon is the lowest-risk of the three ports — it has no market-data store and nothing to migrate. It is not stateless, though: `--mode record` appends `~/.nullclaw/chipcon-history.log`. Plan 0 (`FetchError::Upstream`, `price-store`) is already shipped; this plan needs only the `Upstream` variant.

**Tech Stack:** Rust 2021, `market-fetch`, `claw-core` (path dep into `~/b/gwebcdb/crates/claw-core`).

**Spec:** `docs/specs/2026-07-29-con-family-rust-port-phase3-design.md` (revision 4). Read "The three skills are less alike than rev 1 and rev 2 assumed" and "`chart.error`" before starting.

## Global Constraints

- **The Python is the oracle.** Every ported function is checked against `chipcon/scripts/run.py`. Where this plan's prose and that file disagree, **the file wins** — report the discrepancy rather than editing code to match prose. Two claims about chipcon in an earlier design revision were simply false, so this is not hypothetical.
- **The Python stays in place and stays the cron entry point** until the cutover in Task 6. Do not edit `chipcon/scripts/run.py` or `chipcon/scripts/test_run.py`.
- **This is a translation, not a redesign.** Preserve behaviour exactly, including the parts that look like mistakes; the two known warts are named below and neither is fixed here.
- `cargo test` green in `~/a/claw-skills`; `cargo build --release` succeeds.
- Agents must not run `git commit`, `git add`, `git stash`, `git checkout`, `git restore`, or `cargo fmt`. A human gates every commit.
- `~/a/claw-skills` may carry unrelated uncommitted work. Touch only `crates/chipcon/**` unless a step says otherwise.

### Two warts that are preserved, not fixed

1. **`load_config` runs outside `main`'s `try`.** A malformed or unreadable config therefore produces neither the `[skill-status:...]`/`[trace:...]` markers nor the controlled `CHIPCON failed:` stderr line — it just panics out. The port reproduces this. Fixing it is a separate approved change.
2. **The fallback `deliver_or_fail` cannot accept `parse_mode`.** `run.py` defines a fallback `def deliver_or_fail(deliver_to, output, account="main")` for when the nullclaw libs are absent, but `emit` calls it with `parse_mode=None`, so a manual run outside nullclaw raises `TypeError` instead of printing the report. In Rust `claw-core` is a hard dependency and there is no fallback path, so this wart **cannot** be reproduced and simply disappears. Record it in Task 6 as an intentional difference rather than letting it look like an accident.

### Test code is literal; implementation is by contract

**Test code below is verbatim and must be used as written. Implementation is described by contract; the implementer writes it.** A wrong literal test ships as a bug — every assertion below was written after reading the function it pins.

---

## File Structure

```
crates/chipcon/Cargo.toml
crates/chipcon/src/lib.rs        # module wiring
crates/chipcon/src/analysis.rs   # as_rows..classify — pure, no IO
crates/chipcon/src/render.rs     # format_message, record_line, fmt_price, fmt_pct
crates/chipcon/src/fetch.rs      # update_state — the only network-facing module
crates/chipcon/src/config.rs     # load_config, default_events
crates/chipcon/src/main.rs       # arg parsing, mode dispatch, emit, markers
crates/chipcon/tests/analysis.rs
crates/chipcon/tests/render.rs
crates/chipcon/tests/fetch.rs
crates/chipcon/tests/contract.rs # nullclaw marker/exit goldens
```

The split exists so the pure classification logic — the part with the most behaviour per line — can be tested without touching the network or the clock.

---

### Task 1: the analysis layer

**Files:** Create `crates/chipcon/Cargo.toml`, `src/lib.rs`, `src/analysis.rs`, `tests/analysis.rs`. Modify the workspace `Cargo.toml` members.

**Interfaces:**
- Produces:
  - `pub struct Row { pub day: String, pub close: f64 }`
  - `pub fn pct(a: f64, b: f64) -> f64`
  - `pub fn ma(rows: &[Row], n: usize) -> Option<f64>`
  - `pub fn ma_rising(rows: &[Row], n: usize, lookback: usize) -> Option<bool>`
  - `pub fn return_n(rows: &[Row], n: usize) -> Option<f64>`
  - `pub fn consecutive_down(rows: &[Row]) -> usize`
  - `pub enum Status { Ok, Yellow, Orange, Red, ProfitProtect, InsufficientHistory }`
  - `pub struct Details { … }` carrying every key the Python `details` dict has: `day, current, ma20, ma50, rising20, smh5, qqq5, soxx5, rel_qqq5, rel_soxx5, down_days, distance20, distance50, rows, reasons`
  - `pub fn classify(smh: &[Row], qqq: &[Row], soxx: &[Row]) -> (Status, Details)`

**Contract.** Translate `run.py`'s `as_rows`, `pct`, `ma`, `ma_rising`, `return_n`, `consecutive_down` and `classify` line by line. Read them; do not reconstruct from this description. Three details are easy to lose and each has a test below:

- **`ma_rising` needs `n + lookback` rows**, and compares `ma(rows, n)` against `ma(rows[:-lookback], n)` — the *past* window ends `lookback` rows earlier, it is not a shifted average of the same span.
- **`return_n` needs `len > n`**, not `>= n`.
- **RED and ORANGE accumulate reasons; YELLOW does not.** In the Python the RED block is three independent `if`s and the ORANGE block is three independent `if`s, so several reasons can attach. The YELLOW block is `if / elif / elif`, so at most one attaches. Preserving that asymmetry matters because `reasons` is rendered to the user.

- [ ] **Step 1: Write the failing tests**

`crates/chipcon/tests/analysis.rs`:
```rust
use chipcon::analysis::{classify, consecutive_down, ma, ma_rising, pct, return_n, Row, Status};

/// Ascending series of `n` rows, close = base + i*step. Dates are only labels here.
fn series(n: usize, base: f64, step: f64) -> Vec<Row> {
    (0..n).map(|i| Row { day: format!("d{i:03}"), close: base + step * i as f64 }).collect()
}

fn flat(n: usize, close: f64) -> Vec<Row> {
    (0..n).map(|i| Row { day: format!("d{i:03}"), close }).collect()
}

#[test]
fn insufficient_history_below_twenty_rows() {
    let (s, d) = classify(&series(19, 100.0, 1.0), &[], &[]);
    assert_eq!(s, Status::InsufficientHistory);
    assert_eq!(d.rows, 19);
}

#[test]
fn twenty_rows_is_enough_to_classify() {
    let (s, _) = classify(&series(20, 100.0, 1.0), &[], &[]);
    assert_ne!(s, Status::InsufficientHistory);
}

#[test]
fn ma_needs_n_rows() {
    assert!(ma(&series(4, 10.0, 0.0), 5).is_none());
    assert_eq!(ma(&flat(5, 10.0), 5), Some(10.0));
}

#[test]
fn ma_rising_needs_n_plus_lookback_rows() {
    // The boundary the Python encodes as `len(rows) < n + lookback`.
    assert!(ma_rising(&series(24, 100.0, 1.0), 20, 5).is_none(), "24 rows is one short of 20+5");
    assert!(ma_rising(&series(25, 100.0, 1.0), 20, 5).is_some(), "25 rows is exactly enough");
}

#[test]
fn ma_rising_compares_against_a_window_ending_lookback_rows_earlier() {
    // Not a shifted average of the same span: past = ma(rows[:-lookback], n).
    let rising = series(30, 100.0, 1.0);
    assert_eq!(ma_rising(&rising, 20, 5), Some(true));
    let falling: Vec<Row> = rising.iter().rev()
        .enumerate().map(|(i, r)| Row { day: format!("d{i:03}"), close: r.close }).collect();
    assert_eq!(ma_rising(&falling, 20, 5), Some(false));
}

#[test]
fn return_n_needs_strictly_more_than_n_rows() {
    assert!(return_n(&series(5, 100.0, 1.0), 5).is_none(), "len == n must be None");
    assert!(return_n(&series(6, 100.0, 1.0), 5).is_some());
}

#[test]
fn return_n_is_percent_from_n_rows_ago() {
    let rows = vec![
        Row { day: "d0".into(), close: 100.0 },
        Row { day: "d1".into(), close: 110.0 },
    ];
    assert!((return_n(&rows, 1).unwrap() - 10.0).abs() < 1e-9);
}

#[test]
fn consecutive_down_counts_only_the_trailing_run() {
    let rows = vec![
        Row { day: "d0".into(), close: 100.0 },
        Row { day: "d1".into(), close:  90.0 },  // down
        Row { day: "d2".into(), close:  95.0 },  // up — breaks the run
        Row { day: "d3".into(), close:  94.0 },  // down
        Row { day: "d4".into(), close:  93.0 },  // down
    ];
    assert_eq!(consecutive_down(&rows), 2);
    assert_eq!(consecutive_down(&flat(5, 10.0)), 0, "equal closes are not down days");
}

#[test]
fn pct_is_relative_change_in_percent() {
    assert!((pct(110.0, 100.0) - 10.0).abs() < 1e-9);
    assert!((pct(90.0, 100.0) + 10.0).abs() < 1e-9);
}

#[test]
fn red_when_below_50dma() {
    let mut rows = series(60, 100.0, 1.0);
    rows.last_mut().unwrap().close = 50.0;   // far under both averages
    let (s, d) = classify(&rows, &[], &[]);
    assert_eq!(s, Status::Red);
    assert!(d.reasons.iter().any(|r| r.contains("50DMA")), "{:?}", d.reasons);
}

#[test]
fn red_accumulates_every_reason_that_fires() {
    // The RED block is three independent `if`s, not a chain — a run that trips
    // more than one must report more than one.
    let mut rows = series(60, 100.0, 1.0);
    for r in rows.iter_mut().skip(55) { r.close = 40.0; }  // drags 20DMA under 50DMA too
    let (s, d) = classify(&rows, &[], &[]);
    assert_eq!(s, Status::Red);
    assert!(d.reasons.len() >= 2, "expected several RED reasons, got {:?}", d.reasons);
}

/// A series that reaches YELLOW with TWO of its three conditions true.
///
/// Getting here is fiddly and the obvious construction does not work. `current <
/// ma20` together with `rising20 == false` is ORANGE, not YELLOW, so the only
/// route is `current >= ma20` (a bounce) with a still-falling 20DMA — and a
/// bounce makes smh5 strongly positive, so QQQ has to be given an even larger
/// 5-day gain to land rel_qqq5 in (-4, -2]. Solved numerically, not guessed:
///   low 70 → peak 110 over 51 rows, an 8-row dip to 0.86×peak, then a bounce
///   70% of the way back. current 105.380, ma20 104.104, ma50 95.402,
///   rising20 false, rel_qqq5 exactly -3.000, down_days 0.
fn yellow_two_conditions() -> (Vec<Row>, Vec<Row>) {
    let (low, peak, dip_len, dip_to, bounce) = (70.0_f64, 110.0_f64, 8usize, 0.86_f64, 0.70_f64);
    let n_rise = 60 - dip_len - 1;
    let mut smh: Vec<f64> = (0..n_rise)
        .map(|i| low + (peak - low) * i as f64 / (n_rise - 1) as f64)
        .collect();
    for i in 0..dip_len {
        smh.push(peak - (peak - peak * dip_to) * (i + 1) as f64 / dip_len as f64);
    }
    let last = *smh.last().unwrap();
    smh.push(last + (peak - last) * bounce);

    let smh5 = (smh[59] / smh[54] - 1.0) * 100.0;
    let q5 = smh5 + 3.0;                      // rel_qqq5 == -3.0 by construction
    let mut qqq = vec![100.0_f64; 55];
    for i in 1..=5 {
        qqq.push(100.0 * (1.0 + q5 / 100.0 * i as f64 / 5.0));
    }
    let row = |v: &Vec<f64>| -> Vec<Row> {
        v.iter().enumerate().map(|(i, c)| Row { day: format!("d{i:03}"), close: *c }).collect()
    };
    (row(&smh), row(&qqq))
}

#[test]
fn yellow_attaches_exactly_one_reason() {
    // The YELLOW block is if/elif/elif. This fixture makes the second AND third
    // conditions true, so an elif chain yields one reason and three independent
    // ifs yield two. Without a two-condition fixture the distinction is
    // unobservable and the test is decoration.
    let (smh, qqq) = yellow_two_conditions();
    let (s, d) = classify(&smh, &qqq, &[]);
    assert_eq!(s, Status::Yellow, "fixture must reach YELLOW; got {s:?} reasons {:?}", d.reasons);
    assert!(d.rising20 == Some(false), "second condition must hold");
    assert!(d.rel_qqq5.unwrap() <= -2.0, "third condition must hold too: {:?}", d.rel_qqq5);
    assert_eq!(d.reasons.len(), 1, "YELLOW is elif-chained: {:?}", d.reasons);
}

#[test]
fn ok_when_the_trend_is_intact() {
    let rows = series(60, 100.0, 1.0);
    let (s, d) = classify(&rows, &rows, &rows);
    assert_eq!(s, Status::Ok);
    assert!(d.reasons.is_empty(), "{:?}", d.reasons);
}

#[test]
fn yellow_when_underperforming_qqq_by_two_percent() {
    let smh = series(60, 100.0, 0.10);
    let qqq = series(60, 100.0, 1.00);   // QQQ far stronger over the last 5
    let (s, d) = classify(&smh, &qqq, &[]);
    assert!(matches!(s, Status::Yellow | Status::Orange | Status::Red), "got {s:?}");
    assert!(d.rel_qqq5.unwrap() < 0.0);
}

#[test]
fn profit_protect_needs_extension_and_a_down_day() {
    // status still OK, >= 8% above the 20DMA, and at least one down day.
    let mut rows = flat(60, 100.0);
    for r in rows.iter_mut().skip(58) { r.close = 130.0; }
    rows.last_mut().unwrap().close = 129.0;   // one down day, still far extended
    let (s, d) = classify(&rows, &[], &[]);
    assert_eq!(s, Status::ProfitProtect, "reasons {:?} distance20 {:?}", d.reasons, d.distance20);
    assert!(d.distance20.unwrap() >= 8.0);
    assert!(d.down_days >= 1);
}

#[test]
fn extension_without_a_down_day_stays_ok() {
    // The falsifier for PROFIT_PROTECT's `down_days >= 1`. Without this case,
    // deleting that condition changes nothing any test can see, because the
    // profit-protect fixture already has a down day.
    let mut rows = flat(60, 100.0);
    rows.last_mut().unwrap().close = 130.0;    // 28% above the 20DMA, but rising
    let (s, d) = classify(&rows, &[], &[]);
    assert_eq!(s, Status::Ok, "no down day means no PROFIT_PROTECT: {:?}", d.reasons);
    assert!(d.distance20.unwrap() >= 8.0, "the extension condition alone does hold");
    assert_eq!(d.down_days, 0);
}

#[test]
fn details_carries_every_field_the_message_renders() {
    let rows = series(60, 100.0, 1.0);
    let (_, d) = classify(&rows, &rows, &rows);
    assert!(d.ma20.is_some() && d.ma50.is_some());
    assert!(d.distance20.is_some() && d.distance50.is_some());
    assert!(d.smh5.is_some() && d.qqq5.is_some());
    assert_eq!(d.rows, 60);
    assert_eq!(d.day, "d059");
}

#[test]
fn missing_secondary_series_leaves_relatives_none_without_panicking() {
    // update_state hands an empty vec for a ticker whose fetch failed.
    let rows = series(60, 100.0, 1.0);
    let (_, d) = classify(&rows, &[], &[]);
    assert!(d.qqq5.is_none() && d.rel_qqq5.is_none());
    assert!(d.soxx5.is_none() && d.rel_soxx5.is_none());
}
```

- [ ] **Step 2: Run to verify it fails** — the crate does not exist yet.
- [ ] **Step 3: Create the crate and implement to the contract.** `mkdir -p crates/chipcon/{src,tests}` **before** adding the workspace member; cargo hard-errors on a member whose directory is absent. Translate the seven functions from `run.py`.
- [ ] **Step 4: Run to verify it passes** — 18 tests.
- [ ] **Step 5: Mutation gate.** Apply each, confirm **at least** the named test goes red, revert:
  1. ~~`ma_rising` guard `n + lookback` → `n`~~ — **withdrawn, not falsifiable.** That
     outer guard is fully redundant for every call site: `past = ma(rows[:-lookback], n)`
     already needs `len - lookback >= n`, so for `n <= len < n + lookback` the inner
     `past is None` check returns `None` anyway. Measured over lengths 18–27 with
     `n=20, lookback=5`: original and mutated agree on every input. `ma_rising_needs_
     n_plus_lookback_rows` still earns its place — it pins the boundary — but no
     mutation of the outer guard alone can turn it red, and listing one implies
     coverage that does not exist. (The guard is not dead code in general: Python's
     `rows[:-0]` is the empty slice, so `lookback = 0` would diverge. chipcon only
     ever passes 5.)
  2. `return_n` guard `> n` → `>= n` → `return_n_needs_strictly_more_than_n_rows`.
  3. Make the RED block an if/else-if chain → `red_accumulates_every_reason_that_fires`.
  4. Make the YELLOW block three independent ifs → `yellow_attaches_exactly_one_reason`.
  5. `consecutive_down` counts `<=` instead of `<` → `consecutive_down_counts_only_the_trailing_run`.
  6. Drop the `down_days >= 1` condition from PROFIT_PROTECT → `extension_without_a_down_day_stays_ok`. **Not** `profit_protect_needs_extension_and_a_down_day` — that fixture already has a down day, so the mutation leaves it green. A mutation aimed at a test that cannot observe it is worse than no mutation, because it reads as coverage.
  A mutation that leaves the suite green is a **missing test** — report it rather than moving on.

  **Two mutations in earlier drafts of this plan were not falsifiable**, both because
  they aimed at a condition another guard already covered. When a mutation stays
  green, the first question is whether the mutation is observable at all, not whether
  the test is weak. Getting that backwards adds a test that pins nothing.
- [ ] **Step 6: Report**, including any place `run.py` disagreed with this plan's prose.

---

### Task 2: rendering and the history line

**Files:** Create `src/render.rs`, `src/config.rs`, `tests/render.rs`.

**Interfaces:**
- Produces: `fmt_price`, `fmt_pct`, `pub fn format_message(status, details, cfg, warning) -> (String, SkillStatus)`, `pub fn record_line(status, details, warning, now: &str) -> String`, `pub fn default_events() -> Vec<String>`, `pub struct Config { pub symbols: Vec<(String, String)>, pub position_label: String, pub manual_events: Vec<String> }`

**`symbols` must preserve insertion order — not a `BTreeMap`.** Python iterates
`cfg["symbols"]` in dict insertion order (SMH, QQQ, SOXX); a `BTreeMap` iterates
alphabetically (QQQ, SMH, SOXX). On the non-fatal path the difference is invisible
because removing SMH leaves QQQ before SOXX either way — which is exactly why a
`.contains()`-style test would never catch it. On the **fatal** path, where every
warning is joined into the `CHIPCON failed: …` stderr line that nullclaw stores in
`cron_runs.output`, the two orders produce different human-readable text. Parse the
JSON object in document order and keep it., `pub fn load_config(path) -> Config`.

**Contract.** Translate `format_message`, `record_line`, `fmt_price`, `fmt_pct`, `default_events`, `load_config` from `run.py`. Note:

- `format_message` returns `("degraded", …)` when `warning` is present and `("ok", …)` otherwise — the *skill* status, which is **not** the classification status. A `RED` classification with no warning still reports `ok` to nullclaw. That is deliberate: RED is a market signal, not a skill failure.
- `record_line` takes the timestamp as a **parameter** rather than calling the clock, so it can be tested. `main` passes the real CST time. This is the one structural change from the Python and it exists solely for testability.
- The message ends with the literal `SIGNAL-ONLY：這是動能觀測信號，不是交易指令。` — the existing Python test asserts the report is an observation, not an exit review, and that line is the anchor.

- [ ] **Step 1: Write the failing tests**

`crates/chipcon/tests/render.rs`:
```rust
use chipcon::analysis::{Details, Status};
use chipcon::render::{format_message, record_line};

fn details() -> Details {
    Details {
        day: "2026-07-29".into(), current: 250.0,
        ma20: Some(240.0), ma50: Some(230.0), rising20: Some(true),
        smh5: Some(1.5), qqq5: Some(2.0), soxx5: Some(1.0),
        rel_qqq5: Some(-0.5), rel_soxx5: Some(0.5),
        down_days: 1, distance20: Some(4.17), distance50: Some(8.70),
        rows: 60, reasons: vec![],
    }
}

#[test]
fn a_warning_makes_the_skill_status_degraded_but_not_the_classification() {
    let (_, s) = format_message(Status::Ok, &details(), &cfg(), Some("yahoo SOXX: no rows"));
    assert_eq!(s, "degraded");
    let (_, s) = format_message(Status::Ok, &details(), &cfg(), None);
    assert_eq!(s, "ok");
}

#[test]
fn a_red_classification_without_a_warning_is_still_skill_status_ok() {
    // RED is a market signal, not a skill failure. Reporting it as degraded
    // would make the scheduler retry a perfectly successful run.
    let (_, s) = format_message(Status::Red, &details(), &cfg(), None);
    assert_eq!(s, "ok");
}

#[test]
fn the_message_is_an_observation_not_an_exit_instruction() {
    let (m, _) = format_message(Status::Red, &details(), &cfg(), None);
    assert!(m.contains("SIGNAL-ONLY"), "{m}");
    assert!(!m.contains("賣出") && !m.contains("SELL"), "must not instruct: {m}");
}

#[test]
fn insufficient_history_renders_the_row_count_not_the_indicators() {
    let mut d = details();
    d.rows = 12;
    let (m, _) = format_message(Status::InsufficientHistory, &d, &cfg(), None);
    assert!(m.contains("12 / 20 needed"), "{m}");
    assert!(!m.contains("50DMA"), "indicator block must be skipped: {m}");
}

#[test]
fn reasons_are_listed_and_their_absence_says_trend_intact() {
    let mut d = details();
    d.reasons = vec!["SMH below 50DMA".into(), "20DMA below 50DMA".into()];
    let (m, _) = format_message(Status::Red, &d, &cfg(), None);
    assert!(m.contains("- SMH below 50DMA") && m.contains("- 20DMA below 50DMA"), "{m}");
    let (m, _) = format_message(Status::Ok, &details(), &cfg(), None);
    assert!(m.contains("trend intact"), "{m}");
}

#[test]
fn record_line_takes_the_clock_as_a_parameter() {
    let l = record_line(Status::Red, &details(), None, "2026-07-29 05:30:00 CST");
    assert!(l.starts_with("2026-07-29 05:30:00 CST CHIPCON RED "), "{l}");
    assert!(l.contains("SMH=250.00"), "{l}");
    assert!(l.ends_with("warning=-"), "no warning renders as a dash: {l}");
}

#[test]
fn record_line_for_insufficient_history_reports_rows_only() {
    let mut d = details();
    d.rows = 3;
    let l = record_line(Status::InsufficientHistory, &d, Some("boom"), "2026-07-29 05:30:00 CST");
    assert!(l.contains("CHIPCON INSUFFICIENT_HISTORY rows=3"), "{l}");
    assert!(l.ends_with("warning=boom"), "{l}");
    assert!(!l.contains("ma20="), "indicator fields must be omitted: {l}");
}
```

`fn cfg()` returns a `Config` with the three real symbols and the five real `manual_events` from `chipcon/config.json`; write it in the test file from that file's actual contents.

- [ ] **Step 2–4:** run red, implement, run green — 7 tests.
- [ ] **Step 5: Mutation gate.**
  1. Return `"degraded"` for a RED classification → `a_red_classification_without_a_warning_is_still_skill_status_ok`.
  2. Render the indicator block for `InsufficientHistory` → `insufficient_history_renders_the_row_count_not_the_indicators`.
  3. Drop the `warning=-` dash when there is no warning → `record_line_takes_the_clock_as_a_parameter`.
- [ ] **Step 6: Report.**

---

### Task 3: the fetch layer

**Files:** Create `src/fetch.rs`, `tests/fetch.rs`.

**Interfaces:** `pub fn update_state(cfg: &Config, fetch: &dyn Fn(&str) -> Result<Vec<Row>, FetchError>) -> Result<(BTreeMap<String, Vec<Row>>, Option<String>), String>`

Taking the fetcher as a parameter is the seam; the production caller passes one built on `market_fetch::yahoo`. Without it the failure branches cannot be tested without a network.

**Contract.** Translate `update_state`. Per symbol: fetch 1y, sort ascending by date, and on any error push `yahoo fetch {SYM}: {err}` to warnings and store an empty vec; on an empty result push `yahoo {SYM}: no rows`. After the loop, if `SMH` is missing or empty, **return `Err`** with the joined warnings (or `yahoo: no SMH history (primary symbol)`); otherwise return the state and the joined warnings as `Some` if any.

**The `chart.error` mapping.** Plan 0 added `FetchError::Upstream`. Python's `parse_chart_response` returns `[]` — not an exception — when `chart.error` is set, when `chart.result` is absent, or when `closes` is falsy. Rust returns `Upstream` for the first and `NoData` for the others. So the adapter this task builds maps **both `Upstream` and `NoData` to an empty vec**, which then produces the `yahoo {SYM}: no rows` warning — the same text the Python emits. `Http` and `Parse` stay errors and produce `yahoo fetch {SYM}: …`. Mapping only `Upstream` would silently change the warning text for a payload shape Yahoo actually sends.

- [ ] **Step 1: Write the failing tests**

`crates/chipcon/tests/fetch.rs`:
```rust
use chipcon::fetch::{update_state, yahoo_rows_or_empty};
use chipcon::analysis::Row;
use market_fetch::yahoo::FetchError;

fn rows(n: usize) -> Vec<Row> {
    (0..n).map(|i| Row { day: format!("2026-07-{:02}", i + 1), close: 100.0 + i as f64 }).collect()
}

#[test]
fn success_sorts_ascending_by_date() {
    let mut unsorted = rows(3);
    unsorted.reverse();
    let (state, warn) = update_state(&cfg(), &|_| Ok(unsorted.clone())).unwrap();
    assert!(warn.is_none());
    let smh = &state["SMH"];
    assert_eq!(smh[0].day, "2026-07-01");
    assert_eq!(smh[2].day, "2026-07-03");
}

#[test]
fn a_failing_primary_symbol_is_a_hard_error() {
    let e = update_state(&cfg(), &|_| Err(FetchError::Http("boom".into()))).unwrap_err();
    assert!(e.contains("SMH"), "{e}");
}

#[test]
fn an_empty_primary_symbol_is_a_hard_error() {
    let e = update_state(&cfg(), &|_| Ok(vec![])).unwrap_err();
    assert!(e.contains("SMH"), "{e}");
}

#[test]
fn a_failing_secondary_symbol_only_warns() {
    let (state, warn) = update_state(&cfg(), &|sym| {
        if sym == "SMH" { Ok(rows(30)) } else { Err(FetchError::Http("boom".into())) }
    }).unwrap();
    assert_eq!(state["SMH"].len(), 30);
    assert!(state["QQQ"].is_empty());
    let w = warn.expect("a secondary failure must warn");
    assert!(w.contains("yahoo fetch QQQ:"), "{w}");
}

#[test]
fn an_empty_secondary_symbol_warns_with_the_no_rows_wording() {
    let (_, warn) = update_state(&cfg(), &|sym| {
        if sym == "SMH" { Ok(rows(30)) } else { Ok(vec![]) }
    }).unwrap();
    let w = warn.expect("an empty secondary must warn");
    assert!(w.contains("yahoo QQQ: no rows"), "wording must match the Python: {w}");
}

#[test]
fn upstream_and_no_data_both_become_an_empty_series() {
    // Python's parser returns [] for chart.error AND for a missing result or
    // falsy closes. Both must land on the "no rows" warning, not the
    // "fetch failed" one, or the message text silently changes.
    assert!(yahoo_rows_or_empty(Err(FetchError::Upstream("Not Found".into()))).unwrap().is_empty());
    assert!(yahoo_rows_or_empty(Err(FetchError::NoData)).unwrap().is_empty());
}

#[test]
fn warnings_follow_config_order_not_alphabetical_order() {
    // Python iterates the symbols dict in insertion order: SMH, QQQ, SOXX.
    // A BTreeMap would give QQQ, SMH, SOXX. The difference only shows when
    // several warnings are joined — which happens on the fatal path, where the
    // joined text becomes the `CHIPCON failed: …` line nullclaw stores.
    let e = update_state(&cfg(), &|_| Ok(vec![])).unwrap_err();
    let smh = e.find("SMH").expect("SMH warning missing");
    let qqq = e.find("QQQ").expect("QQQ warning missing");
    let soxx = e.find("SOXX").expect("SOXX warning missing");
    assert!(smh < qqq && qqq < soxx, "config order, not alphabetical: {e}");
}

#[test]
fn load_config_preserves_document_order_from_the_file() {
    // The order test above pins "given an ordered Config, warnings follow it".
    // It does NOT pin that load_config PRODUCES an ordered Config, because the
    // cfg() helper builds one directly. Order survives only because Cargo.toml
    // enables serde_json's `preserve_order` feature — and a feature flag is the
    // easiest thing in a manifest to drop during a dependency cleanup. Measured
    // 2026-07-29: removing that feature left all 33 tests green. This test is
    // what makes the flag load-bearing instead of decorative.
    let dir = std::env::temp_dir().join(format!("chipcon-cfg-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.json");
    std::fs::write(&path, r#"{"symbols":{"SMH":"SMH","QQQ":"QQQ","SOXX":"SOXX"}}"#).unwrap();
    let cfg = chipcon::config::load_config(&path);
    let keys: Vec<&str> = cfg.symbols.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(keys, vec!["SMH", "QQQ", "SOXX"], "document order, not alphabetical");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn transport_and_parse_failures_stay_errors() {
    assert!(yahoo_rows_or_empty(Err(FetchError::Http("timeout".into()))).is_err());
    assert!(yahoo_rows_or_empty(Err(FetchError::Parse("bad json".into()))).is_err());
}
```

`fn cfg()` returns a `Config` whose `symbols` are the three real ones.

- [ ] **Step 2–4:** run red, implement, run green — 9 tests.
- [ ] **Step 5: Mutation gate.**
  0a. Store `symbols` in a `BTreeMap` → `warnings_follow_config_order_not_alphabetical_order`.
  0b. Remove `features = ["preserve_order"]` from `serde_json` in `crates/chipcon/Cargo.toml`
      → `load_config_preserves_document_order_from_the_file`. **Verified 2026-07-29 that
      without this test the feature can be removed with all 33 tests still green** — the
      flag was decorative until something asserted on it.
  1. Map only `Upstream` to empty, letting `NoData` through as an error → `upstream_and_no_data_both_become_an_empty_series`.
  2. Map `Http` to empty as well → `transport_and_parse_failures_stay_errors`.
  3. Treat a failing secondary as fatal → `a_failing_secondary_symbol_only_warns`.
  4. Drop the sort → `success_sorts_ascending_by_date`.
- [ ] **Step 6: Report.**

---

### Task 4: main, markers, and the nullclaw contract

**Files:** Create `src/run.rs`, `src/main.rs`, `tests/contract.rs`.

**Interfaces:**
- Produces `pub fn run(argv: &[String], env: &Env, fetch: &dyn Fn(&str) -> Result<Vec<Row>, FetchError>, now: &str, out: &mut dyn Write, err: &mut dyn Write) -> i32`
- `src/main.rs` is a thin wrapper: real argv, real environment, a `market-fetch` fetcher, the real CST clock, `std::io::stdout()`/`stderr()`, then `std::process::exit`.

Threading the writers and the environment through is the seam. Without it none of the
goldens below can be asserted, and the marker contract is precisely the part where
Phase ① showed that being almost right makes a successful run look like a failure.

**Contract, read from `lib/trace_marker.py` and `lib/delivery.py` rather than assumed:**

- `emit_skill_status(status)` prints `[skill-status:<status>]` **only when
  `NULLCLAW_JOB_ID` is set** — it is a no-op otherwise, so a manual run emits no
  markers at all. `emit_trace()` prints `[trace:<NULLCLAW_JOB_ID>]` under the same
  condition. Both go to **stdout**.
- Valid statuses are exactly `ok`, `degraded`, `failed`; anything else raises.
- `deliver_or_fail(None, body, …)` prints the body to stdout and returns success —
  it does not call Telegram. That is how a run with no `--deliver-to` still leaves
  the report in `cron_runs.output`.
- `emit` appends the job id as `\n\n{job_id}` — **unquoted**, unlike oilcon which
  wraps it in backticks — then delivers, **then** emits the two markers in that order.
- record mode never calls `emit`: it appends `record_line` to the history log, then
  emits `degraded` if there was a warning else `ok`, and returns 0. A warned record is
  **accepted**, unlike oilcon which rejects one.
- the error path prints `CHIPCON failed: {e}` to **stderr**, emits `failed`, returns **1**.
- `load_config` runs **before** the try and keeps that position — see the warts section.

- [ ] **Step 1: Write the failing tests**

`crates/chipcon/tests/contract.rs`:
```rust
use chipcon::run::{run, Env};
use chipcon::analysis::Row;
use market_fetch::yahoo::FetchError;

fn rows(n: usize) -> Vec<Row> {
    (0..n).map(|i| Row { day: format!("2026-07-{:02}", i + 1), close: 100.0 + i as f64 }).collect()
}

/// Every symbol succeeds with enough history to classify.
fn good(_sym: &str) -> Result<Vec<Row>, FetchError> { Ok(rows(60)) }

fn env(job: Option<&str>, home: &std::path::Path) -> Env {
    Env { job_id: job.map(String::from), home: home.to_path_buf() }
}

fn tmp() -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("chipcon-c-{}-{:?}", std::process::id(), std::thread::current().id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn go(argv: &[&str], job: Option<&str>, f: &dyn Fn(&str) -> Result<Vec<Row>, FetchError>)
    -> (i32, String, String, std::path::PathBuf)
{
    let home = tmp();
    let a: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    let (mut o, mut e) = (Vec::new(), Vec::new());
    let code = run(&a, &env(job, &home), f, "2026-07-29 05:30:00 CST", &mut o, &mut e);
    (code, String::from_utf8(o).unwrap(), String::from_utf8(e).unwrap(), home)
}

#[test]
fn markers_come_after_the_body_and_in_status_then_trace_order() {
    let (code, out, err, _) = go(&["chipcon"], Some("job-77"), &good);
    assert_eq!(code, 0);
    assert!(err.is_empty(), "stderr must be quiet on success: {err}");
    let body = out.find("CHIPCON").expect("body missing");
    let status = out.find("[skill-status:ok]").expect("status marker missing");
    let trace = out.find("[trace:job-77]").expect("trace marker missing");
    assert!(body < status, "body must precede the markers");
    assert!(status < trace, "skill-status must precede trace");
}

#[test]
fn the_job_id_is_appended_unquoted() {
    // oilcon wraps it in backticks; chipcon does not, and the difference is
    // visible in the delivered message.
    let (_, out, _, _) = go(&["chipcon"], Some("job-77"), &good);
    assert!(out.contains("\n\njob-77"), "job id must be appended bare: {out}");
    assert!(!out.contains("`job-77`"), "chipcon must not quote the job id: {out}");
}

#[test]
fn no_job_id_means_no_markers_at_all() {
    // emit_skill_status and emit_trace are both no-ops when NULLCLAW_JOB_ID is
    // unset, so a manual run must not pollute stdout with marker lines.
    let (code, out, _, _) = go(&["chipcon"], None, &good);
    assert_eq!(code, 0);
    assert!(out.contains("CHIPCON"), "the report itself still prints");
    assert!(!out.contains("[skill-status:"), "no status marker without a job id: {out}");
    assert!(!out.contains("[trace:"), "no trace marker without a job id: {out}");
}

#[test]
fn a_secondary_failure_is_degraded_but_still_delivers_and_exits_zero() {
    let f = |sym: &str| -> Result<Vec<Row>, FetchError> {
        if sym == "SMH" { Ok(rows(60)) } else { Err(FetchError::Http("boom".into())) }
    };
    let (code, out, err, _) = go(&["chipcon"], Some("job-77"), &f);
    assert_eq!(code, 0, "a degraded run is not a failure");
    assert!(out.contains("[skill-status:degraded]"), "{out}");
    assert!(out.contains("[WARN:"), "the warning must reach the reader: {out}");
    assert!(err.is_empty(), "{err}");
}

#[test]
fn a_primary_failure_writes_stderr_emits_failed_and_exits_one() {
    let f = |_: &str| -> Result<Vec<Row>, FetchError> { Err(FetchError::Http("boom".into())) };
    let (code, out, err, _) = go(&["chipcon"], Some("job-77"), &f);
    assert_eq!(code, 1, "a hard failure must exit non-zero");
    assert!(err.starts_with("CHIPCON failed: "), "stderr prefix: {err:?}");
    assert!(out.contains("[skill-status:failed]"), "{out}");
    assert!(out.contains("[trace:job-77]"), "{out}");
}

#[test]
fn record_mode_writes_the_history_line_before_the_markers() {
    let (code, out, _, home) = go(&["chipcon", "--mode", "record"], Some("job-77"), &good);
    assert_eq!(code, 0);
    let log = home.join(".nullclaw/chipcon-history.log");
    let text = std::fs::read_to_string(&log).expect("history log not written");
    assert!(text.starts_with("2026-07-29 05:30:00 CST CHIPCON "), "{text}");
    assert!(text.ends_with('\n'), "the line must be newline-terminated: {text:?}");
    assert!(out.contains("[skill-status:ok]") && out.contains("[trace:job-77]"), "{out}");
    assert!(!out.contains("CHIPCON 情報"), "record mode must not render the report: {out}");
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn record_mode_accepts_a_warned_run_and_reports_degraded() {
    // chipcon and inflation-con record a warned run; oilcon rejects one. Porting
    // oilcon's rule here would silently drop history rows.
    let f = |sym: &str| -> Result<Vec<Row>, FetchError> {
        if sym == "SMH" { Ok(rows(60)) } else { Ok(vec![]) }
    };
    let (code, out, _, home) = go(&["chipcon", "--mode", "record"], Some("job-77"), &f);
    assert_eq!(code, 0);
    let log = home.join(".nullclaw/chipcon-history.log");
    let text = std::fs::read_to_string(&log).expect("a warned run must still be recorded");
    assert!(text.contains("warning=yahoo QQQ: no rows"), "{text}");
    assert!(out.contains("[skill-status:degraded]"), "{out}");
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn record_mode_appends_rather_than_truncating() {
    let home = tmp();
    let log = home.join(".nullclaw/chipcon-history.log");
    std::fs::create_dir_all(log.parent().unwrap()).unwrap();
    std::fs::write(&log, "PRIOR LINE\n").unwrap();
    let a: Vec<String> = ["chipcon", "--mode", "record"].iter().map(|s| s.to_string()).collect();
    let (mut o, mut e) = (Vec::new(), Vec::new());
    run(&a, &env(Some("job-77"), &home), &good, "2026-07-29 05:30:00 CST", &mut o, &mut e);
    let text = std::fs::read_to_string(&log).unwrap();
    assert!(text.starts_with("PRIOR LINE\n"), "history must be appended, not replaced: {text}");
    assert_eq!(text.lines().count(), 2, "{text}");
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn without_deliver_to_the_body_still_reaches_stdout() {
    // deliver_or_fail(None, …) echoes to stdout. That is how a run with no chat
    // configured still leaves its report in cron_runs.output.
    let (code, out, _, _) = go(&["chipcon"], Some("job-77"), &good);
    assert_eq!(code, 0);
    assert!(out.contains("SIGNAL-ONLY"), "the full report must be on stdout: {out}");
}
```

- [ ] **Step 2: Run to verify it fails** — `run` does not exist yet.
- [ ] **Step 3: Implement.** Translate `parse_args`, `emit` and `main` from `run.py` into `run.rs`; `main.rs` is the thin wrapper. **If a golden here disagrees with the Python, the Python wins and the disagreement is a finding.**
- [ ] **Step 4: Run to verify it passes** — 9 tests.
- [ ] **Step 5: Mutation gate.** Apply each, confirm **at least** the named test goes red, revert:
  1. Emit the markers before delivery → `markers_come_after_the_body_and_in_status_then_trace_order`.
  2. Emit `trace` before `skill-status` → same test.
  3. Wrap the job id in backticks → `the_job_id_is_appended_unquoted`.
  4. Emit markers even when the job id is absent → `no_job_id_means_no_markers_at_all`.
  5. Reject a warned record the way oilcon does → `record_mode_accepts_a_warned_run_and_reports_degraded`.
  6. Return 0 on the error path → `a_primary_failure_writes_stderr_emits_failed_and_exits_one`.
  7. Open the history log with truncate instead of append → `record_mode_appends_rather_than_truncating`.
  If a mutation stays green, ask first whether it is observable at all — two mutations in earlier drafts of this plan aimed at conditions another guard already covered.
- [ ] **Step 6: Report**, including any place `run.py` disagreed with the goldens above.

---

### Task 5: differential check against the Python — ✅ DONE 2026-07-31

**Files:** `crates/chipcon/tests/differential.rs` + `crates/chipcon/fixtures/`.

**Result: seven fixture sets, all byte-identical** on message, record line and skill
status — `live` (real Yahoo, classifies RED) plus one synthesised set for each of the
six `Status` values. The classification printed for each set is the **Python's own**,
so the coverage claim is the oracle's, not a directory label. chipcon is now 44 tests;
the workspace is 244.

Verified rather than accepted:

- **The differential runs the Python.** `differential.rs` spawns `python3` at test
  time and parses its stdout; nothing is compared against a stored expectation, and
  `drive_python.py` calls `update_state` / `classify` / `format_message` /
  `record_line` on the loaded module rather than reimplementing any of them. It
  contains zero rendered-string literals.
- **The test can fail.** Two mutations of `render.rs` — a space appended to the
  message title, and the record line's `SMH={:.2}` widened to `{:.3}` — each turned
  it red with exit 101. A differential that only prints DIFFER without failing would
  be worthless.
- **No network, with a positive control.** Zero `connect()` and zero `AF_INET` in
  both the Python driver and the Rust test binary under `strace`; a real HTTPS
  request under the same filter shows 10 of each, so the zero is a measurement and
  not a filter that missed. Note a blackhole proxy proves nothing here — the test
  strips proxy variables from the child environment, as oilcon's does.
- **The live fixture is genuinely live**: `SMH`, exchange `NGM`, `ETF`, USD, 251
  points spanning 2025-07-31 to 2026-07-30, closes 283.95–668.91, 98% non-integer.

**The trap this task contains**, recorded because it nearly cost the implementer the
run: chipcon has no `cst_now`-style helper. `record_line` calls
`datetime.now(tz).strftime(...)` **inline** (`run.py:248`), so the driver has to
substitute the `datetime` **class**, not a function. The real class must be captured
**once at module load** — capturing it per fixture set means the second set grabs
`FakeDT` and `FakeDT(...)` raises `TypeError`.

**Not covered here** (all fixtures fetch successfully and warn nowhere): the degraded
text from a failed secondary fetch, a hard `update_state` failure, `--mode record`
writing its file, and delivery/markers. Those remain on the contract and fetch tests.

Run both implementations over the **same captured Yahoo payloads** with the clock frozen, and compare the rendered message and the record line. Capture the fixtures once from the real endpoint and commit them; do not hit the network in the test.

This is the step that catches what unit tests cannot: a faithful-looking port whose arithmetic drifts in the third decimal, or whose reason list orders differently.

- [ ] **Step 1:** capture one payload per symbol into `crates/chipcon/fixtures/`.
- [ ] **Step 2:** drive the Python **without editing it**. `update_state` calls
  `oil_fetch.fetch_history` through the module object, so a separate driver can
  replace it — exactly what `chipcon/scripts/test_run.py` already does with
  `monkeypatch.setattr(run, "oil_fetch", ...)`. Write
  `crates/chipcon/fixtures/drive_python.py` that imports `run`, substitutes a stub
  reading the captured fixtures, calls `run.main(["--mode", "record"])`, and prints
  the history line. `chipcon/scripts/run.py` stays untouched.
- [ ] **Step 3:** run the Rust binary against the same fixtures with the clock
  pinned to the same string; diff the record line **and** the full rendered
  message plus its skill status. The message is what the cron actually delivers,
  so comparing only the history line would leave the user-visible output
  unchecked.
- [ ] **Step 4:** report any difference, however small, before deciding whether it is acceptable.

---

### Task 6: cutover and documentation

**Why this task is written out rather than left to judgement.** On 2026-07-30 both
already-ported skills were found running **stale binaries**: `weather` and `doughcon`
were each deployed 2026-07-28 22:24, while their sources had been rebuilt on 07-29.
The weather double-delivery fix (`ca73e83`) was committed, tested, and *not running*.
Its acceptance criteria said "`cargo build --release` succeeds (weather ships as a
binary at `~/.nullclaw/skills/weather/bin/weather`)" — the path was named in a
parenthetical and never became a step. Compiling is not deploying, and a green build
plus a green soak monitor together still cannot tell you which version ran.

Both `SKILL.md` files also still show `python3 …/bin/weather` in their Usage block —
Python-era text that survived the port. It does not break the scheduler (nullclaw
reads the `## Script` block and `resolveInterpreterPrefix` runs a non-`.py` file
directly) but it breaks anyone following the documented example by hand.

- [ ] **Step 1: Build and record the artifact's identity**

```bash
cd ~/a/claw-skills && cargo build --release -p chipcon
sha256sum target/release/chipcon
```
Keep that hash. It is the only thing that later proves what got deployed.

- [ ] **Step 2: Preflight against the Python, no delivery**

Run the Rust binary with `--deliver-to` omitted and compare its rendered message to
the Python's for the same day. Task 5's differential already proves byte-equality on
captured fixtures; this repeats it against whatever the live endpoint returns today,
which is the case fixtures cannot cover.

- [ ] **Step 3: Deploy — a copy, then a proof**

```bash
install -m 755 target/release/chipcon ~/.nullclaw/skills/chipcon/bin/chipcon
sha256sum ~/.nullclaw/skills/chipcon/bin/chipcon
```
**The two hashes must match.** This step exists because it was the missing one:
without it, "deployed" is an assumption. **This is a change to a live system and is
the human's call — prepare the command and stop.**

- [ ] **Step 4: Fix the `SKILL.md` Usage block in the same change**

`## Script` points at the binary; the Usage examples must invoke it directly, with no
`python3` prefix. Do this in the same commit as the cutover, not later — the two
already-ported skills show what "later" means in practice.

- [ ] **Step 5: Switch the scheduler**

One `SKILL.md` line. **Human's call.** After it fires once, confirm from the run that
the deployed hash is still the one from Step 3 — a rebuild between deploy and switch
would silently substitute a different binary.

- [ ] **Step 6: Acceptance and rollback**

**Accept** after 3 consecutive scheduled runs (Tue–Sat 05:30) where the marker status
matches the Python's for the same inputs and the history line has the same shape.
Not "status=ok" alone: a legitimate upstream warning is a `degraded` run and a
correct one.

**Rollback triggers**, not just the action: classification differs from the Python on
identical inputs; a Rust-only non-zero exit; marker text, ordering or exit code
differing from the goldens; or a history line whose shape changed. **Rollback** is
reverting the `SKILL.md` line — the Python entry point stays in place throughout.

- [ ] **Step 7: Record the intentional differences**

- The fallback `deliver_or_fail` `TypeError` disappears because Rust has no fallback
  path — `claw-core` is a hard dependency. Not an accident; state it.
- `record_line` takes the clock as a parameter, for testability.
- `Upstream` and `NoData` both map to an empty series, and the warning text depends
  on that mapping.
- `Config.symbols` preserves document order, which needs `serde_json`'s
  `preserve_order` feature; `load_config_preserves_document_order_from_the_file` is
  what keeps that feature load-bearing rather than decorative.

- [ ] **Step 8: Report**, including both hashes from Steps 1 and 3.

---

## Test Plan

| Layer | What | Gate |
|---|---|---|
| L1 | analysis — 18 tests | every Task 1 mutation turns at least its named test red |
| L2 | render — 7 tests | likewise |
| L3 | fetch — 9 tests | likewise; the `Upstream`/`NoData` pair is the one that protects the warning text |
| L4 | contract — 9 goldens | markers, ordering, exit codes, record mode, append-not-truncate |
| L5 | differential vs Python on captured payloads | any difference reported before it is accepted |

## Acceptance Criteria

1. All tests green; `cargo test` green across `~/a/claw-skills`.
2. Every listed mutation turns at least its named test red; results recorded.
3. The differential produces an identical record line for the same fixtures, or every difference is reported and explicitly accepted.
4. `chipcon/scripts/run.py` and its tests are unmodified.
5. The `SKILL.md` switch is prepared but not applied by an agent.

## Out of Scope

inflation-con and oilcon (Plans 2 and 3), fixing either wart, and retiring the Python.
