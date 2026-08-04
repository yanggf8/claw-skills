# cds-con readability v2 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rewrite the cds-con daily message from a space-aligned table into a vertical, self-explanatory form whose percentiles are printed as counts, and split the daily set from the monthly set.

**Architecture:** All rendering stays in `crates/cds-con/src/render.rs`. The counting primitive is added to the shared `credit-store` crate **additively**, so the single-implementation rule holds and `price-cli` is untouched. The daily/monthly split is a filter applied to the *rendered* set after `analyze()`, driven by a config-supplied day bound evaluated against the injected `as_of` date — never a wall clock.

**Tech Stack:** Rust, `libsql`, `claw-core::delivery`, `credit-store`. Tests are plain `cargo test` (`tests/render.rs`, `tests/contract.rs`).

## Global Constraints

Copied verbatim from `docs/specs/2026-08-04-cds-con-readability-v2-design.md`:

- **No verdict.** No `狀態：` line, no status ladder, no cheap/expensive/wide/narrow label. Every change must survive the reverse-day test: on a day the numbers move the other way it must not become false or become a judgment.
- **`SIGNAL-ONLY` stays**, marker and prose both. `closes_with_signal_only_and_has_no_status_line` must stay green.
- **`parse_mode: None` stays.** Do not switch to HTML/`<pre>`. All 17 contract tests must stay green.
- **A percentile must never carry a `%` sign.** `percent_marks_values_but_never_percentiles` stays unchanged and green. No share/percentage is printed beside a count.
- **Wording is 「低於」, never 「不高於」** — `credit-store/src/stats.rs:26` is `filter(|v| **v < x)`, strictly below.
- **The display may never claim a higher rank than the data supports.**
- **No hardcode.** The monthly-expansion day bound lives in the registry `config` table, not in Rust.
- **No clock inside `render`/`run`.** The date is the injected `as_of` (`main.rs` supplies CST via `cst_today()`).
- **Two blocks stay separate**, all windows stay side by side, per-line coverage stays, the freshness line stays and does not judge.
- Repo: `~/a/claw-skills`. Sibling crate: `~/b/gwebcdb/crates/credit-store`.

---

### Task 1: Counting primitive in `credit-store`

Render needs `below` and `n`, not a percentage. Adding it here rather than in `render.rs` keeps one comparison site.

**Files:**
- Modify: `~/b/gwebcdb/crates/credit-store/src/stats.rs`
- Test: `~/b/gwebcdb/crates/credit-store/src/stats.rs` (in-file `mod tests`)

**Interfaces:**
- Consumes: existing `window_start`, `Observation`.
- Produces: `pub fn below_and_total(values: &[f64], x: f64) -> (usize, usize)` and `pub enum WindowCounts { Computed { below: usize, n: usize }, Insufficient { need: String, have: String } }` plus `pub fn window_counts(rows: &[Observation], years: u32) -> WindowCounts`. **`WindowStat` and `percentile_rank` keep their current signatures** so `price-cli` compiles unchanged.

- [ ] **Step 1: Write the failing test**

Append to the `mod tests` in `stats.rs`:

```rust
#[test]
fn below_and_total_counts_strictly_below() {
    let v = vec![1.0, 2.0, 2.0, 3.0];
    // 2.0 appears twice; neither counts as "below" itself.
    assert_eq!(below_and_total(&v, 2.0), (1, 4));
    assert_eq!(below_and_total(&v, 1.0), (0, 4));
    assert_eq!(below_and_total(&v, 9.0), (4, 4));
}

#[test]
fn below_and_total_agrees_with_percentile_rank() {
    let v = vec![5.0, 1.0, 3.0, 9.0, 3.0];
    for x in [1.0, 3.0, 5.0, 9.0, 0.5, 100.0] {
        let (below, n) = below_and_total(&v, x);
        assert_eq!(
            100.0 * below as f64 / n as f64,
            percentile_rank(&v, x),
            "the two must never disagree: they are one comparison"
        );
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd ~/b/gwebcdb/crates/credit-store && cargo test below_and_total`
Expected: FAIL — `cannot find function `below_and_total``

- [ ] **Step 3: Write minimal implementation**

In `stats.rs`, add above `percentile_rank` and rewrite `percentile_rank` to delegate:

```rust
/// Count of observations strictly below `x`, and the total. The single
/// comparison site: `percentile_rank` is this divided out, so the two can
/// never disagree.
pub fn below_and_total(values: &[f64], x: f64) -> (usize, usize) {
    let below = values.iter().filter(|v| **v < x).count();
    (below, values.len())
}

/// Percentile rank of `x` within `values`: the share of observations strictly below it.
/// Reported as a rank, not a distance from a mean — extremes do not "pull" it.
pub fn percentile_rank(values: &[f64], x: f64) -> f64 {
    if values.is_empty() {
        return f64::NAN;
    }
    let (below, n) = below_and_total(values, x);
    100.0 * below as f64 / n as f64
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd ~/b/gwebcdb/crates/credit-store && cargo test`
Expected: PASS, and every pre-existing test still green.

- [ ] **Step 5: Write the failing window test**

```rust
#[test]
fn window_counts_reports_counts_and_refuses_short_coverage() {
    let rows = vec![
        Observation { date: "2020-01-01".into(), value: 1.0 },
        Observation { date: "2025-06-01".into(), value: 5.0 },
        Observation { date: "2026-01-01".into(), value: 3.0 },
    ];
    match window_counts(&rows, 1) {
        WindowCounts::Computed { below, n } => {
            // Window is 2025-01-01..; rows are 5.0 and 3.0; latest is 3.0.
            assert_eq!((below, n), (0, 2));
        }
        other => panic!("expected Computed, got {other:?}"),
    }
    match window_counts(&rows, 10) {
        WindowCounts::Insufficient { .. } => {}
        other => panic!("10y must be insufficient, got {other:?}"),
    }
}
```

- [ ] **Step 6: Run it to see it fail**

Run: `cargo test window_counts`
Expected: FAIL — `cannot find function `window_counts``

- [ ] **Step 7: Implement `window_counts`**

Mirror `window_stat` exactly, changing only what it reports. Read the existing `window_stat` body and reuse its `window_start` call and its `Insufficient` construction verbatim, so the two can never diverge on which rows are in the window:

```rust
/// Outcome of asking for a trailing-window count.
#[derive(Debug, PartialEq)]
pub enum WindowCounts {
    Computed { below: usize, n: usize },
    Insufficient { need: String, have: String },
}

/// Counts within a trailing `years` window. `rows` must be ascending by date.
/// Same window arithmetic and same refusal as [`window_stat`].
pub fn window_counts(rows: &[Observation], years: u32) -> WindowCounts {
    match window_stat(rows, years) {
        WindowStat::Insufficient { need, have } => WindowCounts::Insufficient { need, have },
        WindowStat::Computed { n, .. } => {
            let last = rows.last().expect("window_stat returned Computed on empty rows");
            let start = window_start(&last.date, years);
            let vals: Vec<f64> = rows
                .iter()
                .filter(|r| r.date >= start)
                .map(|r| r.value)
                .collect();
            let (below, total) = below_and_total(&vals, last.value);
            debug_assert_eq!(total, n, "window_counts and window_stat must select the same rows");
            WindowCounts::Computed { below, n: total }
        }
    }
}
```

If `window_start` is private, make it `pub(crate)` — do not duplicate its leap-day clamping.

- [ ] **Step 8: Run tests**

Run: `cd ~/b/gwebcdb/crates/credit-store && cargo test`
Expected: PASS. Then `cd ~/b/gwebcdb && cargo build -p price-cli` — expected: builds unchanged.

- [ ] **Step 9: Export and commit**

Add `below_and_total`, `window_counts`, `WindowCounts` to the crate's public re-exports in `lib.rs` alongside `window_stat`.

```bash
cd ~/b/gwebcdb
git add crates/credit-store/src/stats.rs crates/credit-store/src/lib.rs
git commit -m "feat(credit-store): report window counts, not only the percentile

cds-con prints 'N/M 筆低於本次' rather than pN, which needs the count
itself. percentile_rank now delegates to below_and_total so there is
still exactly one comparison site."
```

---

### Task 2: Windows render as counts

**Files:**
- Modify: `~/a/claw-skills/crates/cds-con/src/render.rs`
- Test: `~/a/claw-skills/crates/cds-con/tests/render.rs`

**Interfaces:**
- Consumes: `credit_store::{window_counts, WindowCounts, below_and_total}` from Task 1.
- Produces: `pub struct WindowPct { pub label: String, pub below: usize, pub n: usize }` — **`label` becomes `String`, not `&'static str`**, because the full-history label is now a computed year. `windows_str` is deleted; `window_lines(&SeriesLine) -> Vec<String>` replaces it.

- [ ] **Step 1: Write the failing tests**

Add to `tests/render.rs`:

```rust
#[test]
fn wording_is_strictly_below_never_at_most() {
    let msg = render_lines(&golden_lines(), "2026-07-31");
    assert!(msg.contains("筆低於本次"), "must state 低於");
    assert!(!msg.contains("不高於"), "不高於 is <=, the implementation is <");
}

#[test]
fn zero_below_renders_as_zero_over_n() {
    // A series sitting at its window minimum must print 0/N, never a blank,
    // an omitted window, or a dash. This is the p0 ambiguity fix.
    let line = SeriesLine {
        key: "q".into(),
        label: "Q".into(),
        kind: SeriesKind::Spread,
        value: Some(0.43),
        windows: vec![WindowPct { label: "近1年".into(), below: 0, n: 13 }],
        coverage_start: Some("1919-01-01".into()),
        latest: Some("2026-07-01".into()),
        frequency: Frequency::Monthly,
        config_order: 0,
    };
    let msg = render_lines(&[line], "2026-07-31");
    assert!(msg.contains("0/13 筆低於本次"), "got:\n{msg}");
}

#[test]
fn no_share_percentage_is_printed_beside_a_count() {
    let msg = render_lines(&golden_lines(), "2026-07-31");
    for l in msg.lines().filter(|l| l.contains("筆低於本次")) {
        assert!(!l.contains('%'), "a count line must carry no %: {l}");
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cd ~/a/claw-skills/crates/cds-con && cargo test --test render wording_is_strictly_below`
Expected: FAIL to compile — `WindowPct` has no field `below`.

- [ ] **Step 3: Implement**

In `render.rs` replace `WindowPct` and `windows_str`, and update `series_line_from_rows` to use `window_counts`:

```rust
/// A trailing-window count that the series can actually support.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowPct {
    /// Display label: `近1年`, `近10年`, or the coverage start year (`自1986`).
    pub label: String,
    /// Observations in the window strictly below the latest value.
    pub below: usize,
    /// Observations in the window.
    pub n: usize,
}

/// One line per window. A percentile is not printed; the count is the
/// definition, so the reader never has to know what `p` meant.
fn window_lines(line: &SeriesLine) -> Vec<String> {
    let w = line
        .windows
        .iter()
        .map(|w| display_width(&w.label))
        .max()
        .unwrap_or(0);
    let c = line
        .windows
        .iter()
        .map(|w| format!("{}/{}", w.below, w.n).len())
        .max()
        .unwrap_or(0);
    line.windows
        .iter()
        .map(|win| {
            let pair = format!("{}/{}", win.below, win.n);
            format!(
                "  {}  {:>cw$} 筆低於本次",
                pad_to(&win.label, w),
                pair,
                cw = c
            )
        })
        .collect()
}
```

In `series_line_from_rows`, replace the `window_stat` loop:

```rust
    for (years, label) in [(1u32, "近1年"), (10u32, "近10年")] {
        if let WindowCounts::Computed { below, n } = window_counts(rows, years) {
            windows.push(WindowPct { label: label.to_string(), below, n });
        }
    }

    let vals: Vec<f64> = rows.iter().map(|r| r.value).collect();
    let (below, n) = below_and_total(&vals, last.value);
    windows.push(WindowPct {
        label: format!("自{}", &rows[0].date[..4.min(rows[0].date.len())]),
        below,
        n,
    });
```

Note `pad_to`/`display_width` are still used here — they are removed in Task 4, at which point `window_lines` switches to plain `format!` with no padding. Keeping them one more task keeps this task's diff small and its test honest.

- [ ] **Step 4: Run tests**

Run: `cargo test --test render`
Expected: the three new tests PASS. `golden_message_matches_plan_exactly` and `cjk_labels_keep_columns_aligned` now FAIL — expected; they are deleted in Task 4 and Task 6. Every other test passes.

- [ ] **Step 5: Commit**

```bash
cd ~/a/claw-skills
git add crates/cds-con/src/render.rs crates/cds-con/tests/render.rs
git commit -m "feat(cds-con): print window counts instead of pN

'61/250 筆低於本次' explains the definition in place, matches
credit-store's strictly-below comparison, and removes the p0 ambiguity
where truncation mapped all of [0,1) onto the same label."
```

---

### Task 3: The full-history window is labelled by its start year

Task 2 already emits `自1986`. This task deletes the now-dead `全庫` path and `coverage_str`'s duplication, and pins that three different coverages produce three different labels.

**Files:**
- Modify: `~/a/claw-skills/crates/cds-con/src/render.rs`
- Test: `~/a/claw-skills/crates/cds-con/tests/render.rs`

**Interfaces:**
- Consumes: `WindowPct` from Task 2.
- Produces: `fn coverage_year(line: &SeriesLine) -> Option<&str>` returning the 4-char year, used by both the series meta line and the footer contrast in Task 5.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn window_label_is_the_actual_start_year() {
    let msg = render_lines(&golden_lines(), "2026-07-31");
    assert!(!msg.contains("全庫"), "全庫 hides that each row is a different ruler");
    assert!(msg.contains("自1919") || msg.contains("自1986"), "got:\n{msg}");
}

#[test]
fn three_series_with_different_coverage_get_three_different_labels() {
    let msg = render_lines(&golden_lines(), "2026-07-31");
    let years: std::collections::HashSet<&str> = msg
        .lines()
        .filter(|l| l.contains("筆低於本次"))
        .filter_map(|l| l.split_whitespace().find(|t| t.starts_with('自')))
        .collect();
    assert!(
        years.len() >= 3,
        "the golden spans 1919/1986/2023; each must print its own ruler, got {years:?}"
    );
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --test render window_label_is_the_actual_start_year`
Expected: PASS already if Task 2 landed cleanly; if `全庫` still appears anywhere, FAIL. Run `three_series_...` too — expected PASS. **If both already pass, that is the correct outcome**; this task then only removes dead code and adds the pins. Do not weaken the tests to manufacture a failure.

- [ ] **Step 3: Remove the dead constant and centralise the year**

```rust
/// Coverage start year, the only part of a start date that carries meaning:
/// it is what makes a 1-year window over a 3-year history read differently
/// from one over 107 years.
fn coverage_year(line: &SeriesLine) -> Option<&str> {
    line.coverage_start.as_deref().map(|s| &s[..4.min(s.len())])
}
```

Rewrite `coverage_str` to use it:

```rust
fn coverage_str(line: &SeriesLine) -> String {
    let freq = match line.frequency {
        Frequency::Daily => "日頻",
        Frequency::Monthly => "月頻",
    };
    match coverage_year(line) {
        Some(y) => format!("{freq}・自{y}"),
        None => format!("{freq}・自—"),
    }
}
```

Delete the `"全庫"` literal from `series_line_from_rows` (Task 2 already replaced its use).

- [ ] **Step 4: Run tests**

Run: `cargo test --test render`
Expected: both new tests PASS; no test regresses beyond the two known failures from Task 2.

- [ ] **Step 5: Commit**

```bash
git add crates/cds-con/src/render.rs crates/cds-con/tests/render.rs
git commit -m "feat(cds-con): label the full window by its start year

全庫 printed identically for a 107-year, a 40-year and a 3-year history,
hiding the one difference the message works hardest to expose."
```

---

### Task 4: Vertical layout; the column machinery is deleted

**Files:**
- Modify: `~/a/claw-skills/crates/cds-con/src/render.rs`
- Test: `~/a/claw-skills/crates/cds-con/tests/render.rs`
- Delete from tests: `cjk_labels_keep_columns_aligned`

**Interfaces:**
- Consumes: `window_lines` (Task 2), `coverage_str` (Task 3).
- Produces: `fn series_block(line: &SeriesLine) -> Vec<String>`. **`display_width`, `pad_to`, `RowWidths`, `row_widths`, `format_series_row` are deleted.** `WIDTH_BOUND: usize = 40` is a render-module constant used only by tests via a `pub fn width_bound() -> usize`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn every_rendered_line_fits_its_width_bound() {
    // Proxy, not proof: the transport is a proportional font, where this
    // model does not describe the real wrap point. What it prevents is lines
    // bloating back to a size that breaks even a monospace reader.
    // Covers EVERY non-blank line — a series title at 80 columns wraps too.
    fn width(s: &str) -> usize {
        s.chars()
            .map(|c| {
                let c = c as u32;
                let wide = (0x1100..=0x115F).contains(&c)
                    || (0x2E80..=0xA4CF).contains(&c)
                    || (0xAC00..=0xD7A3).contains(&c)
                    || (0xF900..=0xFAFF).contains(&c)
                    || (0xFE30..=0xFE6F).contains(&c)
                    || (0xFF00..=0xFF60).contains(&c)
                    || (0xFFE0..=0xFFE6).contains(&c);
                if wide { 2 } else { 1 }
            })
            .sum()
    }
    let msg = render_lines(&golden_lines(), "2026-07-31");
    for l in msg.lines().filter(|l| !l.trim().is_empty()) {
        // Prose lines (block headers, SIGNAL-ONLY footer) may wrap harmlessly.
        if l.starts_with("利差") || l.starts_with("總殖利率")
            || l.starts_with("SIGNAL-ONLY") || l.starts_with("資料:")
            || l.starts_with("月頻") || l.starts_with("自") && l.ends_with("尺。")
        {
            continue;
        }
        assert!(width(l) <= 40, "line is {} cols: {l}", width(l));
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --test render every_rendered_line_fits`
Expected: FAIL — current rows are 85 columns.

- [ ] **Step 3: Implement the vertical block**

```rust
/// One series as a block of lines. No padding across series: the transport
/// renders a proportional font, so column alignment was never reaching the
/// reader — see the 2026-08-04 design note.
fn series_block(line: &SeriesLine) -> Vec<String> {
    let mut out = vec![format!("{} [{}]", line.label, line.key)];
    out.push(format!("  {}  {}", value_str(line), coverage_str(line)));
    out.extend(window_lines(line));
    out
}
```

Rewrite `render_lines`' body to push `series_block(line)` per series with a blank line between, and **delete** `display_width`, `pad_to`, `RowWidths`, `row_widths`, `format_series_row`, and `label_str`. Change `window_lines` to drop its `pad_to` call now that alignment is abandoned:

```rust
fn window_lines(line: &SeriesLine) -> Vec<String> {
    line.windows
        .iter()
        .map(|w| format!("  {}  {}/{} 筆低於本次", w.label, w.below, w.n))
        .collect()
}
```

- [ ] **Step 4: Delete the obsolete alignment test**

Remove `cjk_labels_keep_columns_aligned` from `tests/render.rs` entirely. It asserted that byte offsets differ while display columns match — meaningless once nothing is padded.

- [ ] **Step 5: Run tests**

Run: `cargo test --test render`
Expected: `every_rendered_line_fits_its_width_bound` PASSES; only `golden_message_matches_plan_exactly` still fails (fixed in Task 6).

- [ ] **Step 6: Commit**

```bash
git add crates/cds-con/src/render.rs crates/cds-con/tests/render.rs
git commit -m "refactor(cds-con): vertical layout, delete the column machinery

parse_mode is None, so Telegram renders this in a proportional font where
space padding produces no columns. display_width/pad_to/RowWidths computed
an alignment the reader never saw. Widest data line: 85 cols -> 38."
```

---

### Task 5: Headers, the computed footer contrast, and the daily/monthly split

**Files:**
- Modify: `~/a/claw-skills/crates/cds-con/src/render.rs`
- Modify: `~/a/claw-skills/crates/cds-con/src/run.rs`
- Test: `~/a/claw-skills/crates/cds-con/tests/render.rs`

**Interfaces:**
- Consumes: `coverage_year` (Task 3), `series_block` (Task 4), `read_config` (`run.rs:204`).
- Produces: `pub fn format_message(series: &[SeriesInput], as_of: &str, expand_days: u32) -> Result<String, RenderError>` — **signature gains `expand_days`**. `pub fn render_lines(lines: &[SeriesLine], as_of: &str, expand_days: u32) -> String` likewise. Config key: `cds_monthly_expand_days`, default `7` when the key is absent.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn ordinary_day_carries_exactly_the_six_daily_series() {
    let msg = render_lines(&golden_lines(), "2026-07-31", 7); // day 31 > 7
    assert!(!msg.contains("[baa]"), "monthly series must be collapsed");
    assert!(!msg.contains("[aaa]"));
    assert!(!msg.contains("[baa−aaa]"));
    assert!(msg.contains("[baa10y]") && msg.contains("[ccc_yield]"));
}

#[test]
fn monthly_block_expands_only_within_the_configured_day_bound() {
    let inside = render_lines(&golden_lines(), "2026-08-07", 7);
    let outside = render_lines(&golden_lines(), "2026-08-08", 7);
    assert!(inside.contains("[baa]"), "day 7 must expand");
    assert!(!outside.contains("[baa]"), "day 8 must not");
    // The bound is data: moving it moves the boundary.
    let moved = render_lines(&golden_lines(), "2026-08-08", 10);
    assert!(moved.contains("[baa]"), "bound must come from config, not code");
}

#[test]
fn day_bound_is_evaluated_from_as_of_never_a_clock() {
    // main.rs injects a CST date (cst_today, offset +8). Rendering must be a
    // pure function of as_of, so a UTC-keyed clock cannot creep in and
    // misfire for a whole month while fixed-date tests stay green.
    let a = render_lines(&golden_lines(), "2026-08-07", 7);
    let b = render_lines(&golden_lines(), "2026-08-07", 7);
    assert_eq!(a, b);
    assert!(a.contains("[baa]"));
}

#[test]
fn monthly_status_line_present_whenever_the_block_is_collapsed() {
    let msg = render_lines(&golden_lines(), "2026-07-31", 7);
    assert!(msg.contains("月頻 3 列 資料至 2026-06"), "got:\n{msg}");
    assert!(msg.contains("未展開"));
}

#[test]
fn monthly_status_line_absent_when_expanded() {
    let msg = render_lines(&golden_lines(), "2026-08-07", 7);
    assert!(!msg.contains("未展開"));
    assert!(msg.contains("月 至 2026-06"), "資料 line carries it instead");
}

#[test]
fn monthly_status_line_names_missing_monthly_series() {
    // format_freshness_line finds missing series by scanning RENDERED lines.
    // Filtering the monthly rows out must not hide a missing one for 29 days.
    let mut lines = golden_lines();
    for l in lines.iter_mut() {
        if l.key == "aaa" {
            l.value = None;
            l.windows.clear();
        }
    }
    let msg = render_lines(&lines, "2026-07-31", 7);
    assert!(msg.contains("缺 aaa"), "got:\n{msg}");
}

#[test]
fn footer_contrast_uses_only_series_rendered_today() {
    let msg = render_lines(&golden_lines(), "2026-07-31", 7);
    let shown: Vec<String> = msg
        .lines()
        .filter(|l| l.contains("筆低於本次"))
        .filter_map(|l| l.split_whitespace().find(|t| t.starts_with('自')).map(|s| s.to_string()))
        .collect();
    for tok in msg.lines().last().unwrap().split_whitespace().filter(|t| t.starts_with('自')) {
        assert!(
            shown.iter().any(|s| s == tok),
            "footer names {tok}, which is not on screen today. shown={shown:?}"
        );
    }
}

#[test]
fn spreads_header_makes_no_claim_about_the_price_of_credit_risk() {
    let msg = render_lines(&golden_lines(), "2026-07-31", 7);
    assert!(!msg.contains("信用風險本身的價格"));
    assert!(msg.contains("利差 —— 相對某個基準多出的殖利率"));
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --test render`
Expected: FAIL to compile — `render_lines` takes 2 arguments.

- [ ] **Step 3: Implement**

Add to `render.rs`:

```rust
/// Day-of-month from an injected `YYYY-MM-DD`. No clock: `main.rs` supplies a
/// CST calendar date, so keying the bound on `as_of` is CST by construction.
fn day_of_month(as_of: &str) -> u32 {
    as_of.get(8..10).and_then(|d| d.parse().ok()).unwrap_or(1)
}

/// Monthly series change once a month; on the other ~29 days they do not earn
/// a third of the message. The split is by publication frequency, never by
/// value, so the rule is identical whichever way the market moves.
fn expand_monthly(as_of: &str, expand_days: u32) -> bool {
    day_of_month(as_of) <= expand_days
}
```

In `render_lines`, immediately after computing `lines` (never before `analyze`, or the derived `baa−aaa` stops being built at all):

```rust
    let expand = expand_monthly(as_of, expand_days);
    let shown: Vec<&SeriesLine> = lines
        .iter()
        .filter(|l| expand || l.frequency == Frequency::Daily)
        .collect();
```

Use `shown` for the two blocks and for the footer contrast; keep the **full** `lines` for the monthly status line so a collapsed-but-missing series is still named:

```rust
/// Collapsed monthly summary. Carries the month reached AND any missing
/// monthly series — without the latter, filtering the rows out would hide a
/// missing series for the ~29 days the block is collapsed.
fn monthly_status_line(lines: &[SeriesLine], expand_days: u32) -> Option<String> {
    let monthly: Vec<&SeriesLine> = lines
        .iter()
        .filter(|l| l.frequency == Frequency::Monthly)
        .collect();
    if monthly.is_empty() {
        return None;
    }
    let reached = monthly
        .iter()
        .filter_map(|l| l.latest.as_ref())
        .min()
        .map(|d| d[..7.min(d.len())].to_string())
        .unwrap_or_else(|| "—".into());
    let mut s = format!(
        "月頻 {} 列 資料至 {},未展開(每月 1–{} 日展開)",
        monthly.len(),
        reached,
        expand_days
    );
    let missing: Vec<&str> = monthly
        .iter()
        .filter(|l| l.value.is_none())
        .map(|l| l.key.as_str())
        .collect();
    if !missing.is_empty() {
        s.push_str(&format!("・缺 {}", missing.join(",")));
    }
    Some(s)
}

/// Two rulers that are actually on screen today. Naming a collapsed series'
/// start year would point the reader at something not in the message.
fn footer_contrast(shown: &[&SeriesLine]) -> Option<String> {
    let mut with_cov: Vec<&&SeriesLine> = shown
        .iter()
        .filter(|l| l.coverage_start.is_some() && !l.windows.is_empty())
        .collect();
    with_cov.sort_by_key(|l| l.coverage_start.clone());
    let (first, last) = (with_cov.first()?, with_cov.last()?);
    if coverage_year(first) == coverage_year(last) {
        return None;
    }
    let n = |l: &SeriesLine| l.windows.last().map(|w| w.n).unwrap_or(0);
    Some(format!(
        "自{} 的 {} 筆和自{} 的 {} 筆不是同一把尺。",
        coverage_year(last)?, n(last), coverage_year(first)?, n(first)
    ))
}
```

Block headers become:

```rust
    out.push("利差 —— 相對某個基準多出的殖利率".into());
    ...
    out.push("總殖利率 —— 含無風險利率在內的全部借款成本(與上一區不可互比)".into());
```

Footer:

```rust
    out.push(format_freshness_line(&shown_owned, as_of));
    if let Some(s) = monthly_status_line(lines, expand_days).filter(|_| !expand) {
        out.push(s);
    }
    out.push("SIGNAL-ONLY:每個窗口各自回答自己的問題,不可跨列比較——".into());
    if let Some(c) = footer_contrast(&shown) {
        out.push(c);
    }
```

Delete `window_example` — with counts on every line the demonstration is no longer a separate sentence.

- [ ] **Step 4: Wire the config key in `run.rs`**

```rust
/// Days of the month on which the monthly block expands. Data, not code —
/// absent key means 7.
async fn monthly_expand_days(conn: &Connection) -> u32 {
    read_config(conn, "cds_monthly_expand_days")
        .await
        .ok()
        .flatten()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(7)
}
```

and at the `format_message` call site:

```rust
    let expand_days = monthly_expand_days(conn).await;
    let message = match format_message(&series, as_of, expand_days) {
```

- [ ] **Step 5: Run tests**

Run: `cd ~/a/claw-skills/crates/cds-con && cargo test`
Expected: all render tests PASS except `golden_message_matches_plan_exactly`; **all 17 contract tests PASS**.

- [ ] **Step 6: Commit**

```bash
git add crates/cds-con/src/render.rs crates/cds-con/src/run.rs crates/cds-con/tests/render.rs
git commit -m "feat(cds-con): daily/monthly split, computed footer contrast

Monthly series expand on days 1-7 (bound from config), and the collapsed
status line carries both the month reached and any missing monthly series
so filtering cannot hide one. The footer's ruler contrast is now drawn
from the series actually rendered, not written into a sentence."
```

---

### Task 6: Goldens, config labels, and documentation

**Files:**
- Modify: `~/a/claw-skills/crates/cds-con/tests/render.rs`
- Modify: `~/a/claw-skills/cds-con/SKILL.md`
- Modify: `~/a/claw-skills/docs/specs/2026-08-01-cds-con-intentional-differences.md`

**Interfaces:**
- Consumes: everything above.
- Produces: `golden_ordinary_day`, `golden_first_seven_days`.

- [ ] **Step 1: Replace the old golden test**

Delete `golden_message_matches_plan_exactly`. Add two tests whose constants are the **exact** blocks from the design doc's §「Target output」 (ordinary day) and its days-1–7 variant. Generate them by running the renderer once and eyeballing against the design doc — then paste, so the constant is a decision and not a recording of whatever the code did:

```rust
#[test]
fn golden_ordinary_day() {
    let rendered = render_lines(&golden_lines(), "2026-07-31", 7);
    assert_eq!(rendered, GOLDEN_ORDINARY, "ordinary-day message must match the design doc byte-for-byte");
}

#[test]
fn golden_first_seven_days() {
    let rendered = render_lines(&golden_lines(), "2026-08-07", 7);
    assert_eq!(rendered, GOLDEN_EXPANDED, "days 1-7 message must match the design doc byte-for-byte");
}
```

- [ ] **Step 2: Run the full suite**

Run: `cd ~/a/claw-skills/crates/cds-con && cargo test`
Expected: PASS, all of it. Also `cargo build --release`.

- [ ] **Step 3: Update the live config labels**

The series names are data. Set them to the design doc's §3 wording:

```bash
cd ~/b/gwebcdb
./target/release/price config list | grep cds_series
```

Then rewrite the `Label` field of each of the eight entries (`key|SERIES_ID|Label|kind`) to: `Baa 比 10年期美債多出的殖利率`, `高收益債相對基準多出的殖利率`, `投資級債相對基準多出的殖利率`, `Baa 級公司債總殖利率`, `Aaa 級公司債總殖利率`, `高收益債總殖利率`, `投資級債總殖利率`, `CCC 及以下總殖利率`. Set `BAA_AAA_LABEL` in `render.rs` to `Baa 比 Aaa 多出的殖利率` to match their style.

**Do not invent labels not in the design doc.** No Rust test can see this field — it is a hand-verified step.

- [ ] **Step 4: Update SKILL.md**

Rewrite these sections to describe what now exists, not what used to: 「Message layout: what is data and what is code」 (drop the measured-column-widths paragraph and the `Label [key]` + `%` paragraph; add counts, start-year windows, the daily/monthly split and its config key), and the footer example. Keep the 「no 狀態 line」 section unchanged.

- [ ] **Step 5: Append the decision record**

Add a 「Readability pass v2 (2026-08-04)」 section to `docs/specs/2026-08-01-cds-con-intentional-differences.md` pointing at the design doc, and stating the three things future readers will otherwise re-litigate: why alignment was abandoned rather than fixed, why no share percentage sits beside a count, and that the days-1–7 rule is a proxy that can be wrong in one specific way.

- [ ] **Step 6: Commit**

```bash
cd ~/a/claw-skills
git add crates/cds-con/tests/render.rs cds-con/SKILL.md docs/specs/2026-08-01-cds-con-intentional-differences.md crates/cds-con/src/render.rs
git commit -m "test(cds-con): two goldens for the v2 message; document the pass"
```

---

### Task 7: Cutover

**Files:** none — verification only.

- [ ] **Step 1: Install the built binary**

Run: `cd ~/a/claw-skills && tools/install-skill.sh cds-con`
Expected: exit 0, and its smoke probe (exit 2 on an unknown flag) passes.

- [ ] **Step 2: Run it live against the store**

Run: `~/.nullclaw/skills/cds-con/bin/cds-con`
Expected: exit 0, and the message matches the design doc's ordinary-day shape with today's numbers.

- [ ] **Step 3: Deliver once to the real chat and read it on the phone**

Run: `~/.nullclaw/skills/cds-con/bin/cds-con --deliver-to 7972814626`

**This step is the reason the plan exists.** The width assertion is a proxy; a proportional font has no column model. Confirm on the device that no data line wraps and that the message is readable. The 2026-08-02 pass shipped alignment that was never visible because this check was skipped.

- [ ] **Step 4: Confirm the day-1–7 rule against the calendar**

Verify the run today (day 4) expanded the monthly block, and that the `資料:` line carries `月 至`. If today is past the 7th, verify the collapsed status line appears instead and names the month reached.

- [ ] **Step 5: Leave the cron alone**

Both cron jobs are unchanged (`0 6` fetch, `30 6` deliver, Tue–Sat). Nothing to edit. If the message is wrong on the phone, `git revert` the range and reinstall — cds-con only reads, so there is nothing to unwind.

---

## Self-Review

**Spec coverage.** §1 vertical layout → Task 4. §2 counts, no share → Tasks 1–2. §3 start year + labels + header → Tasks 3, 5, 6 Step 3. §4 daily/monthly split, config bound, CST, status line incl. missing → Task 5. §5 footer rule → Task 5. §6 losses → documentation only, Task 6 Step 5. §7 unchanged items → guarded by the Global Constraints and by the untouched contract suite. Test plan's 14 new tests: `wording_is_strictly_below_never_at_most`, `zero_below_renders_as_zero_over_n`, `no_share_percentage_is_printed_beside_a_count` (Task 2); `window_label_is_the_actual_start_year`, `three_series_with_different_coverage_get_three_different_labels` (Task 3); `every_rendered_line_fits_its_width_bound` (Task 4); `ordinary_day_carries_exactly_the_six_daily_series`, `monthly_block_expands_only_within_the_configured_day_bound`, `day_bound_is_evaluated_from_as_of_never_a_clock`, `monthly_status_line_present_whenever_the_block_is_collapsed`, `monthly_status_line_absent_when_expanded`, `monthly_status_line_names_missing_monthly_series`, `footer_contrast_uses_only_series_rendered_today`, `spreads_header_makes_no_claim_about_the_price_of_credit_risk` (Task 5); two goldens (Task 6). All 14 present.

**Type consistency.** `WindowPct.label` is `String` from Task 2 onward and every later construction uses `.into()`/`format!`. `render_lines` and `format_message` both gain `expand_days: u32` in Task 5, and every test written before Task 5 is updated there — Task 5 Step 2 expects exactly that compile error. `coverage_year` returns `Option<&str>` and is used by `coverage_str` and `footer_contrast` only.

**Known gap, deliberate.** Tasks 2–5 leave `golden_message_matches_plan_exactly` failing until Task 6 deletes it. Each task's expected-result line says so, so a red suite mid-plan is not mistaken for a defect.

---

## Execution Handoff

Plan complete and saved to `docs/plans/2026-08-04-cds-con-readability-v2.md`. Two execution options:

**1. Subagent-Driven (recommended)** — a fresh subagent per task, review between tasks, fast iteration.

**2. Inline Execution** — execute tasks in this session using executing-plans, batch execution with checkpoints.
