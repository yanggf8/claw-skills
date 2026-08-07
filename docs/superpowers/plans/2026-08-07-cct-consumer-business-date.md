# cct Consumer: read the business date — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the `cct` skill read `metadata.business_date` and `metadata.has_content` from the worker envelope, so it stops re-deriving which trading day a report is about and stops guessing whether a report has content.

**Architecture:** The envelope reaches the skill intact instead of being discarded at the unwrap layer. The comparison clock then follows the field that was read — ET for `business_date`, UTC for the legacy `data["date"]` — which is what makes the two repos deployable in either order. `has_content: false` becomes the authoritative empty signal, ahead of the per-mode content predicates that had to infer it.

**Tech Stack:** Rust, `serde_json`, `jiff` (with `tzdb-bundle-always`, so named zones need nothing from the host). Tests are `cargo test -p cct`: unit tests plus a binary-level suite that runs the real executable against a fail-closed TCP stub.

**Source spec:** `~/a/cct/docs/specs/2026-08-07-business-date-envelope-design.md`, §4 "Consumer change". The worker side is implemented and deployed (worker version `e055f340`); all five report routes carry the field in production today.

## Global Constraints

- **The comparison clock follows the field that was read.** `metadata.business_date` is an ET business date and is compared against today in `America/New_York`. The fallback `data["date"]` is what the pre-ET worker served and is compared against today in UTC. Binding them per-field, not globally, is what keeps both deploy orders safe — see the table in Task 3.
- Nothing may reach stdout except the delivered message body and the two scheduler marker lines. Diagnostics go to stderr (`claw_core::delivery` and `claw_core::outcome` own stdout).
- The scheduler contract is unchanged: cct emits only `Ok` or `Degraded`, degraded still delivers, exit code stays 0, markers stay gated on `NULLCLAW_JOB_ID`.
- `cargo clippy --workspace --all-targets` must report zero warnings; `tools/lint-http.sh` must stay green.
- Publish only with `tools/install-skill.sh cct` — never by hand. The installer's smoke probe requires an unknown flag to exit 2.
- Do not weaken `freshness.rs`'s 32 existing tests to fit; they pin behaviour that is still correct.

## File Structure

| File | Responsibility | Change |
|---|---|---|
| `crates/cct/src/api.rs` | Fetch and unwrap the envelope; gains a `Report` return type carrying provenance | Modify |
| `crates/cct/src/main.rs` | Resolve the day, the clock and the gap; wire the rest together | Modify |
| `crates/cct/src/render.rs` | `eod_session_date` prefers the stated day over its guess chain | Modify |
| `crates/cct/tests/envelope.rs` | Unwrap-layer tests, updated for the new return type | Modify |
| `crates/cct/tests/binary.rs` | End-to-end behaviour through the real binary | Modify |

---

### Task 1: The envelope survives the unwrap

Today `unwrap_envelope` returns `Some(data)` and drops everything around it (`api.rs:111`), so no envelope field can reach the skill however faithfully the worker publishes it. This task changes only the return type; nothing consumes the new fields yet.

**Files:**
- Modify: `crates/cct/src/api.rs`
- Modify: `crates/cct/tests/envelope.rs`

**Interfaces:**
- Produces:
  ```rust
  pub struct Report {
      pub data: serde_json::Value,
      pub business_date: Option<String>,
      pub has_content: Option<bool>,
  }
  ```
  `unwrap_envelope(text: &str, err: &mut impl Write) -> Option<Report>`, `get(path: &str) -> Option<Report>`.

- [ ] **Step 1: Write the failing test**

Append to `crates/cct/tests/envelope.rs`:

```rust
#[test]
fn the_envelopes_provenance_survives_the_unwrap() {
    // The whole point. `unwrap_envelope` returned `Some(data)` and dropped
    // everything around it, so the worker could publish business_date perfectly
    // and the skill would still never see it.
    let (got, warn) = run(
        r#"{"success":true,"data":{"symbols_analyzed":5},
            "metadata":{"business_date":"2026-08-06","has_content":true,"source":"d1_fallback"}}"#,
    );
    let report = got.expect("a payload");
    assert_eq!(report.data["symbols_analyzed"], 5);
    assert_eq!(report.business_date.as_deref(), Some("2026-08-06"));
    assert_eq!(report.has_content, Some(true));
    assert_eq!(warn, "");
}

#[test]
fn an_envelope_without_the_fields_is_still_accepted() {
    // A worker that has not shipped the field yet, which is every deploy order
    // where the skill lands first. Absent is not an error; it selects the
    // fallback path in main.
    let (got, _) = run(r#"{"success":true,"data":{"symbols_analyzed":5}}"#);
    let report = got.expect("a payload");
    assert_eq!(report.business_date, None);
    assert_eq!(report.has_content, None);
}

#[test]
fn a_business_date_that_is_not_a_string_is_treated_as_absent() {
    // Fail soft, not loud: a malformed field must not cost the reader a report
    // it could otherwise have had. The fallback path still works.
    let (got, _) = run(
        r#"{"success":true,"data":{"symbols_analyzed":5},"metadata":{"business_date":20260806}}"#,
    );
    assert_eq!(got.expect("a payload").business_date, None);
}
```

Change the helper at the top of the file so the existing tests keep compiling:

```rust
fn run(text: &str) -> (Option<cct::api::Report>, String) {
    let mut err: Vec<u8> = Vec::new();
    let got = unwrap_envelope(text, &mut err);
    (got, String::from_utf8(err).expect("warnings are utf-8"))
}
```

and update the three existing assertions that index the payload directly:
- `a_healthy_envelope_unwraps_to_its_payload_in_silence`: `got.unwrap()["symbols_analyzed"]` → `got.unwrap().data["symbols_analyzed"]`
- `a_payload_that_merely_omits_success_is_not_caught_by_the_inner_test`: unchanged, it only checks `is_some()`
- `the_incident_envelope_is_rejected_with_a_reason` and the rest assert `is_none()` and are unaffected.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p cct --test envelope`
Expected: compile error — `cct::api::Report` does not exist.

- [ ] **Step 3: Implement**

In `crates/cct/src/api.rs`, add above `unwrap_envelope`:

```rust
/// A report plus what the envelope says about it.
///
/// `unwrap_envelope` used to return the payload alone, which meant no envelope
/// field could reach the skill no matter how faithfully the worker published
/// one. The provenance is the point: `business_date` is the ET trading day the
/// content is about, and `has_content` says whether anything was found for it.
/// Both are `Option` because a worker that predates them is a normal thing to
/// meet — the skill and the worker deploy independently.
pub struct Report {
    pub data: serde_json::Value,
    pub business_date: Option<String>,
    pub has_content: Option<bool>,
}
```

Change the end of `unwrap_envelope` from `Some(data)` to:

```rust
    let metadata = parsed.get("metadata");
    Some(Report {
        data,
        // A non-string date is treated as absent rather than as an error: a
        // malformed field must not cost the reader a report they could have had.
        business_date: metadata
            .and_then(|m| m.get("business_date"))
            .and_then(|v| v.as_str())
            .map(str::to_string),
        has_content: metadata
            .and_then(|m| m.get("has_content"))
            .and_then(|v| v.as_bool()),
    })
```

Change the signature to `-> Option<Report>` in both `unwrap_envelope` and `get`, and in `main.rs` bind `Some(report)` and use `report.data` at the four call sites that currently take `&data` (`format_pre_market`, `format_intraday`, `format_eod`, `format_weekly`, and `content_gap`).

- [ ] **Step 4: Run tests**

Run: `cargo test -p cct`
Expected: all pass, including the 9 pre-existing envelope tests.

Run: `cargo clippy -p cct --all-targets`
Expected: zero warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/cct/src/api.rs crates/cct/src/main.rs crates/cct/tests/envelope.rs
git commit -m "refactor(cct): the envelope's provenance survives the unwrap

unwrap_envelope returned Some(data) and dropped everything around it, so the
worker could publish metadata.business_date perfectly and the skill would never
see it. Nothing reads the new fields yet; this is the layer that made reading
them impossible."
```

---

### Task 2: The two clocks

**Files:**
- Modify: `crates/cct/src/main.rs`
- Modify: `crates/cct/tests/binary.rs`

**Interfaces:**
- Consumes: `Report` from Task 1.
- Produces: nothing new; `main` gains `et_today` alongside `today`.

- [ ] **Step 1: Write the failing test**

**Corrected during execution.** This step first specified two binary-level tests
comparing a rendered verdict against the ET clock. They cannot discriminate: ET
and UTC name the same day for twenty hours out of twenty-four, so outside the
00:00–04:00 UTC window those tests pass whatever the rule is — while reading as
coverage. Noting "confirm it discriminates" in the step was not enough; a test
that can only fail during four hours of the day is one that will be green when
it matters least.

Split in three instead:

1. **The rule, as a function, against fixed dates** — `comparison_today(business_date, et_today, utc_today)` in `freshness.rs`, tested with `et = 2026-08-06` and `utc = 2026-08-07` so the two never coincide. Deterministic at every hour.
2. **The composition, over the source** — the unit tests cannot see whether `main` *uses* the rule, and end-to-end cannot see it either for the same twenty hours. So `binary.rs` asserts `main.rs` derives `today` from `comparison_today(...)` and that nothing downstream of that line reaches for `et_today` or `utc_today` directly. Mutation-verified: replacing the call with either clock turns it red.
3. **The path still works** — one binary test that a report carrying provenance renders and classifies, since the ET branch is new code between the fetch and the render.

```rust
#[test]
fn a_stated_business_date_is_judged_against_the_et_clock() {
    let et: Date = "2026-08-06".parse().unwrap();
    let utc: Date = "2026-08-07".parse().unwrap();
    assert_eq!(cct::freshness::comparison_today(Some("2026-08-06"), et, utc), et);
}

#[test]
fn without_one_the_legacy_utc_clock_is_kept() {
    let et: Date = "2026-08-06".parse().unwrap();
    let utc: Date = "2026-08-07".parse().unwrap();
    assert_eq!(cct::freshness::comparison_today(None, et, utc), utc);
}

#[test]
fn the_choice_is_the_field_and_not_the_value() {
    let et: Date = "2026-08-06".parse().unwrap();
    let utc: Date = "2026-08-07".parse().unwrap();
    assert_eq!(cct::freshness::comparison_today(Some("2020-01-01"), et, utc), et);
}
```

- [ ] **Step 3: Implement**

In `crates/cct/src/main.rs`, replace the single clock:

```rust
    let now = jiff::Timestamp::now().in_tz("UTC").expect("UTC");
    let today = now.date();
```

with both, and resolve per report:

```rust
    let now = jiff::Timestamp::now().in_tz("UTC").expect("UTC");
    let utc_today = now.date();
    // ET is the market's own time, so the trading day IS the ET date. The tz
    // database is bundled into the binary, so this needs nothing from the host.
    let et_today = jiff::Timestamp::now()
        .in_tz("America/New_York")
        .expect("tzdb is bundled")
        .date();
```

and inside the `Some(report)` arm, before rendering:

```rust
            // The clock follows the field. `business_date` is an ET business
            // date; the legacy `data["date"]` is what the pre-ET worker served
            // and is only comparable to UTC. Binding the two per-field, rather
            // than switching globally, is what lets this skill and the worker
            // deploy in either order: whichever the reader gets, it is judged
            // against the clock that produced it.
            let today = if report.business_date.is_some() { et_today } else { utc_today };
```

`format_pre_market`, `format_eod` and `content_gap` all take `today` already, so nothing else changes.

- [ ] **Step 4: Run tests**

Run: `cargo test -p cct && cargo clippy -p cct --all-targets`
Expected: all pass, zero warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/cct/src/main.rs crates/cct/tests/binary.rs
git commit -m "fix(cct): the comparison clock follows the field it read

business_date is an ET business date and data[\"date\"] is what the pre-ET route
served, so one clock cannot judge both. Between 00:00 and ~04:00 UTC a report
correctly dated for the ET session reads a day old to a UTC clock, and the skill
would deliver 資料已過期 over a fresh briefing. Binding the clock to the field's
provenance also makes the two repos deployable in either order."
```

---

### Task 3: `has_content: false` is the answer, not a hint

**Files:**
- Modify: `crates/cct/src/main.rs`
- Modify: `crates/cct/tests/binary.rs`

**Interfaces:**
- Consumes: `Report` (Task 1), the two clocks (Task 2).

- [ ] **Step 1: Write the failing test**

Append to `crates/cct/tests/binary.rs`:

```rust
#[test]
fn the_worker_saying_it_found_nothing_is_taken_at_its_word() {
    // The eod placeholder, as production serves it: a well-formed report about
    // a day for which no analysis exists. The per-mode predicates had to infer
    // that from the payload's shape; the envelope now states it, and the reason
    // names the day so the alert points somewhere.
    let data = r#"{"type":"end_of_day_summary","date":"2026-08-07",
                   "daily_summary":{"symbols_analyzed":0,"key_events":["Market closed"]}}"#;
    let stub = Stub::serving(envelope_with(data, "2026-08-07", false));
    let (stdout, stderr, code) = run_stub(&stub, "eod");
    assert_eq!(code, 0);
    assert!(stdout.contains("[skill-status:degraded]"), "stdout: {stdout}");
    assert!(stderr.contains("2026-08-07"), "the reason must name the day: {stderr:?}");
}

#[test]
fn a_scorecard_the_worker_vouches_for_is_not_second_guessed() {
    // has_content: true does not override the predicates — a payload that says
    // it has content and does not is still degraded. The envelope is trusted
    // for "nothing here", which it knows better than the reader, not for
    // "everything here", which it does not.
    let data = r#"{"type":"end_of_day_summary","daily_summary":{"symbols_analyzed":0}}"#;
    let stub = Stub::serving(envelope_with(data, "2026-08-07", true));
    let (stdout, _, code) = run_stub(&stub, "eod");
    assert_eq!(code, 0);
    assert!(stdout.contains("[skill-status:degraded]"), "stdout: {stdout}");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p cct --test binary`
Expected: `the_worker_saying_it_found_nothing_is_taken_at_its_word` FAILS — stderr names the shape, not the day, because `content_gap` never sees `business_date`.

- [ ] **Step 3: Implement**

In `main.rs`, replace the `content_gap` call with:

```rust
            // `has_content: false` is the worker's own answer about its own
            // storage, so it wins over any predicate the reader could apply to
            // the payload's shape. The reverse does not hold: `true` is not
            // taken as a guarantee, because the reader can still see an empty
            // payload and the predicates are what caught a dead pipeline.
            let gap = match report.has_content {
                Some(false) => Some(format!(
                    "the worker has no {} content for {}",
                    args.mode.slug(),
                    report.business_date.as_deref().unwrap_or("the day requested"),
                )),
                _ => content_gap(args.mode, &report.data, today),
            };
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p cct && cargo clippy -p cct --all-targets`
Expected: all pass, zero warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/cct/src/main.rs crates/cct/tests/binary.rs
git commit -m "feat(cct): has_content: false is the worker's answer, and it wins

The per-mode predicates had to infer 'this day has no analysis' from the shape
of a payload the route synthesised. The worker knows it outright and now says
so. The reverse is deliberately not trusted: has_content: true does not silence
the predicates, because they are what caught a dead pipeline delivering
plausible-looking reports for 50 days."
```

---

### Task 4: The eod header stops laundering a UTC instant into a business date

**Files:**
- Modify: `crates/cct/src/render.rs`
- Modify: `crates/cct/src/main.rs`
- Modify: `crates/cct/tests/freshness.rs`

**Interfaces:**
- Consumes: `Report.business_date`.
- Produces: `eod_session_date(business_date: Option<&str>, data: &Value, now: &str) -> String`, `format_eod(business_date: Option<&str>, data: &Value, now: &str) -> String`.

- [ ] **Step 1: Write the failing test**

Append to `crates/cct/tests/freshness.rs`:

```rust
#[test]
fn the_stated_business_date_beats_the_guess_chain() {
    // `timestamp` is an ISO **UTC** instant, so taking its first ten characters
    // launders a UTC day into a business date — the header would print
    // 2026-08-07 for a session that closed on 2026-08-06. The chain exists only
    // because no field stated the answer. One does now.
    let data = serde_json::json!({
        "timestamp": "2026-08-07T00:16:14.266Z",
        "signalBreakdown": [{"ticker": "AAPL"}],
    });
    assert_eq!(
        cct::render::eod_session_date(Some("2026-08-06"), &data, "2026-08-09"),
        "2026-08-06"
    );
}

#[test]
fn without_a_stated_date_the_old_chain_still_answers() {
    // A worker that has not shipped the field. The fallback is unchanged, so
    // deploy order stays free.
    let data = serde_json::json!({ "date": "2026-08-05" });
    assert_eq!(cct::render::eod_session_date(None, &data, "2026-08-09"), "2026-08-05");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p cct --test freshness`
Expected: compile error — `eod_session_date` takes two arguments.

- [ ] **Step 3: Implement**

In `render.rs`, change the signature and add the preference:

```rust
pub fn eod_session_date(business_date: Option<&str>, data: &serde_json::Value, now: &str) -> String {
    // Stated beats inferred. Everything below is a guess chain that exists only
    // because nothing used to state the answer, and one link in it —
    // `timestamp` — is an ISO UTC instant whose first ten characters are a UTC
    // day, not a business date.
    if let Some(day) = business_date {
        return day.to_string();
    }
    for key in ["date", "_scheduled_date", "timestamp", "marketCloseTime", "generated_at"] {
```

and thread it through `format_eod`:

```rust
pub fn format_eod(business_date: Option<&str>, data: &serde_json::Value, now: &str) -> String {
    let mut lines = vec![
        format!("📊 CCT 收盤報告｜{}", eod_session_date(business_date, data, now)),
```

In `main.rs`, update the call:

```rust
                Mode::Eod => format_eod(
                    report.business_date.as_deref(),
                    &report.data,
                    &now.strftime("%Y-%m-%d").to_string(),
                ),
```

Update the existing `format_eod` / `eod_session_date` call sites in `tests/freshness.rs` to pass `None` as the first argument — they are testing the fallback chain, which is unchanged.

- [ ] **Step 4: Run tests**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets && tools/lint-http.sh`
Expected: all pass, zero warnings, http lint green.

- [ ] **Step 5: Publish and verify against production**

```bash
tools/install-skill.sh cct
NULLCLAW_JOB_ID=manual-check:1 ~/.nullclaw/skills/cct/bin/cct --mode pre-market
NULLCLAW_JOB_ID=manual-check:1 ~/.nullclaw/skills/cct/bin/cct --mode eod
```

Expected: each mode either delivers with `[skill-status:ok]` and a silent stderr, or degrades with a reason naming the mode and the business date. Compare the rendered dates against
`curl -s -H "X-API-Key: yanggf" "https://tft-trading-system.yanggf.workers.dev/api/v1/reports/end-of-day?cb=$(date +%s%N)" | python3 -m json.tool` — the header must equal `metadata.business_date`, not the UTC day.

- [ ] **Step 6: Commit**

```bash
git add crates/cct/src/render.rs crates/cct/src/main.rs crates/cct/tests/freshness.rs
git commit -m "fix(cct): the eod header states the session, it no longer infers it

eod_session_date fell through date, _scheduled_date, timestamp, marketCloseTime,
generated_at and finally now, taking the first ten characters of whichever it
found. timestamp is an ISO UTC instant, so that truncation launders a UTC day
into a business date: a session that closed on 2026-08-06 printed 2026-08-07.
The chain existed only because nothing stated the answer. The worker states it
now, and the chain stays as the fallback for a worker that has not shipped it."
```

---

## Self-Review

**Spec coverage.** §4 of the spec lists three consumer layers: `api.rs` (Task 1), `main.rs:59` and the provenance-follows-timezone table (Task 2), and `freshness.rs` / `render.rs` (Tasks 3 and 4). Spec testing item 7 — one case per row of that table, plus both deploy orders — is Task 2's two cases plus Task 1's `an_envelope_without_the_fields_is_still_accepted`.

**Placeholders.** None: every step names exact files and shows the code.

**Type consistency.** `Report { data, business_date: Option<String>, has_content: Option<bool> }` is used with that shape in Tasks 1–4. `eod_session_date(Option<&str>, &Value, &str) -> String` and `format_eod(Option<&str>, &Value, &str) -> String` match their call sites in `main.rs` and `tests/freshness.rs`.

**Known risk to check during execution.** Task 2's first test is time-dependent: outside the 00:00–04:00 UTC window it passes against both the old and the new code. The step says so and requires the discrimination to be confirmed by temporarily comparing against `utc_date()`. A test that cannot fail is worth less than no test, because it reads as coverage.

**Deliberately not in scope.** Removing `data["date"]` handling, or the guess chain in `eod_session_date`. Both are the fallback that keeps the deploy orders free, and they cost nothing to keep until every worker in play publishes the field.
