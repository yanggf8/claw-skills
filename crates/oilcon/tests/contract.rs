//! Nullclaw marker + mode-dispatch goldens for oilcon.
//!
//! Expectations are taken from oilcon/scripts/run.py `emit_and_exit` (295–303)
//! and `main` (306–341), lib/trace_marker.py (job-id gate), and
//! lib/delivery.py (None → stdout; parse_mode default "Markdown").
//!
//! oilcon differs from chipcon and inflation-con on nine points — these tests
//! pin every one. Do not reuse those ports' goldens without changing each.

use market_fetch::yahoo::FetchError;
use oilcon::analysis::Row;
use oilcon::run::{deliver_options, run, Env};
use price_store::{ensure_schema, upsert};
use std::path::PathBuf;

const NOW: &str = "2026-07-30 22:00";
const NOW_SECS: &str = "2026-07-30 22:00:00 CST";
const TODAY: &str = "2026-07-30";
const JOB: &str = "job-77";

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

async fn mem() -> libsql::Connection {
    let db = libsql::Builder::new_local(":memory:").build().await.unwrap();
    let c = db.connect().unwrap();
    ensure_schema(&c).await.unwrap();
    c
}

fn row(day: &str, close: f64) -> Row {
    Row {
        day: day.into(),
        close,
    }
}

fn parse(s: &str) -> (i32, u32, u32) {
    let mut p = s.split('-');
    (
        p.next().unwrap().parse().unwrap(),
        p.next().unwrap().parse().unwrap(),
        p.next().unwrap().parse().unwrap(),
    )
}

fn civil(ymd: (i32, u32, u32)) -> i64 {
    let (y, m, d) = ymd;
    let y = y as i64;
    let m = m as i64;
    let d = d as i64;
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn from_civil(z: i64) -> (i32, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let y = y + if m <= 2 { 1 } else { 0 };
    (y as i32, m as u32, d as u32)
}

/// 252 rows spanning 365 calendar days so needs_backfill is false.
fn adequate_series() -> Vec<Row> {
    let n = 252usize;
    let span = 365i64;
    let end_days = civil(parse(TODAY));
    let start_days = end_days - span;
    let mut days: Vec<i64> = (0..n)
        .map(|i| start_days + (i as i64 * span) / (n as i64 - 1))
        .collect();
    days[n - 1] = end_days;
    for i in 1..n {
        if days[i] <= days[i - 1] {
            days[i] = days[i - 1] + 1;
        }
    }
    days.into_iter()
        .enumerate()
        .map(|(i, d)| {
            let (yy, mm, dd) = from_civil(d);
            row(&format!("{yy:04}-{mm:02}-{dd:02}"), 60.0 + i as f64)
        })
        .collect()
}

async fn seed_all(conn: &libsql::Connection) {
    let series = adequate_series();
    for ticker in ["CL=F", "BZ=F", "HO=F"] {
        for r in &series {
            upsert(conn, ticker, &r.day, r.close, "yahoo")
                .await
                .unwrap();
        }
    }
}

fn latest_ok(_sym: &str) -> Result<Option<Row>, FetchError> {
    Ok(Some(row(TODAY, 70.0)))
}

fn history_unused(_sym: &str) -> Result<Vec<Row>, FetchError> {
    // Store is pre-seeded adequately; history must not be required.
    Err(FetchError::Http("history should not be called".into()))
}

fn history_fail(_sym: &str) -> Result<Vec<Row>, FetchError> {
    Err(FetchError::Http("boom".into()))
}

fn latest_fail(_sym: &str) -> Result<Option<Row>, FetchError> {
    Err(FetchError::Http("boom".into()))
}

fn env(job: Option<&str>, home: &std::path::Path) -> Env {
    Env {
        job_id: job.map(String::from),
        home: home.to_path_buf(),
    }
}

fn tmp() -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "oilcon-c-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&d).unwrap();
    // Parent of the history log must exist for successful record writes;
    // oilcon does not mkdir. Tests that need the failure path omit this.
    std::fs::create_dir_all(d.join(".nullclaw")).unwrap();
    d
}

async fn go(
    argv: &[&str],
    job: Option<&str>,
    conn: &libsql::Connection,
    history: &dyn Fn(&str) -> Result<Vec<Row>, FetchError>,
    latest: &dyn Fn(&str) -> Result<Option<Row>, FetchError>,
    home: PathBuf,
) -> (i32, String, String, PathBuf) {
    let a: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    let (mut o, mut e) = (Vec::new(), Vec::new());
    let code = run(
        &a,
        &env(job, &home),
        conn,
        history,
        latest,
        NOW,
        NOW_SECS,
        TODAY,
        &mut o,
        &mut e,
    )
    .await;
    (
        code,
        String::from_utf8(o).unwrap(),
        String::from_utf8(e).unwrap(),
        home,
    )
}

async fn go_seeded(
    argv: &[&str],
    job: Option<&str>,
) -> (i32, String, String, PathBuf) {
    let conn = mem().await;
    seed_all(&conn).await;
    let home = tmp();
    go(
        argv,
        job,
        &conn,
        &history_unused,
        &latest_ok,
        home,
    )
    .await
}

// ---------------------------------------------------------------------------
// Markers and delivery order (run.py:295–303)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn deliver_then_status_then_trace_in_that_order() {
    // emit_and_exit: deliver_or_fail → emit_skill_status → emit_trace.
    // The order whose absence in weather caused the 2026-07-30 double-delivery.
    let (code, out, err, home) = go_seeded(&["oilcon"], Some(JOB)).await;
    assert_eq!(code, 0);
    assert!(err.is_empty(), "stderr must be quiet on success: {err}");
    let body = out.find("🛢️ OILCON 情報").expect("body missing");
    let status = out
        .find("[skill-status:ok]")
        .expect("status marker missing");
    let trace = out.find("[trace:job-77]").expect("trace marker missing");
    assert!(body < status, "body must precede the markers: {out}");
    assert!(status < trace, "skill-status must precede trace: {out}");
    let _ = std::fs::remove_dir_all(&home);
}

#[tokio::test]
async fn the_job_id_is_wrapped_in_backticks() {
    // run.py:299  output += f"\n\n`{job_id}`"
    // chipcon/inflation-con append bare; oilcon must quote.
    let (_, out, _, home) = go_seeded(&["oilcon"], Some(JOB)).await;
    assert!(
        out.contains("\n\n`job-77`"),
        "job id must be wrapped in backticks: {out}"
    );
    // Bare form must not appear as the footer (would match chipcon).
    assert!(
        !out.contains("\n\njob-77\n"),
        "job id must not be appended bare: {out}"
    );
    let _ = std::fs::remove_dir_all(&home);
}

#[tokio::test]
async fn no_job_id_means_no_markers_at_all() {
    // lib/trace_marker.py: both helpers no-op when NULLCLAW_JOB_ID is unset.
    let (code, out, _, home) = go_seeded(&["oilcon"], None).await;
    assert_eq!(code, 0);
    assert!(out.contains("🛢️ OILCON 情報"), "the report itself still prints");
    assert!(
        !out.contains("[skill-status:"),
        "no status marker without a job id: {out}"
    );
    assert!(
        !out.contains("[trace:"),
        "no trace marker without a job id: {out}"
    );
    let _ = std::fs::remove_dir_all(&home);
}

#[tokio::test]
async fn without_deliver_to_the_body_still_reaches_stdout() {
    // lib/delivery.py:48–50  chat_id empty/None → print body to stdout.
    let (code, out, _, home) = go_seeded(&["oilcon"], Some(JOB)).await;
    assert_eq!(code, 0);
    assert!(
        out.contains("🛢️ OILCON 情報"),
        "the full report must be on stdout: {out}"
    );
    assert!(out.contains("WTI:"), "WTI line must be present: {out}");
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn parse_mode_stays_at_the_markdown_default() {
    // lib/delivery.py:24  parse_mode: str | None = "Markdown"
    // oilcon never overrides it (unlike chipcon/inflation-con → None).
    // claw-core DeliverOptions::default() is Some("Markdown") — pin the value
    // oilcon actually builds, not an assumption about `..Default::default()`.
    let opts = deliver_options("main");
    assert_eq!(
        opts.parse_mode.as_deref(),
        Some("Markdown"),
        "oilcon must leave parse_mode at its Markdown default for the backticked job id"
    );
}

// ---------------------------------------------------------------------------
// Warning handling — before mode dispatch (run.py:319–326)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn deliver_plus_warning_is_the_three_line_minimal_message() {
    // run.py:321  f"🛢️ OILCON 情報\n[WARN: {snapshot.warning}]\n更新：{cst_now()}"
    // Not the full report.
    let conn = mem().await; // empty → history fail → warning
    let home = tmp();
    let (code, out, err, home) = go(
        &["oilcon"],
        Some(JOB),
        &conn,
        &history_fail,
        &latest_fail,
        home,
    )
    .await;
    assert_eq!(code, 0, "degraded deliver still exits 0");
    assert!(err.is_empty(), "{err}");
    assert!(
        out.contains("[skill-status:degraded]"),
        "warning deliver is degraded: {out}"
    );
    // Exact three-line shape (plus job-id footer and markers).
    assert!(
        out.contains("🛢️ OILCON 情報\n[WARN:"),
        "title then WARN: {out}"
    );
    assert!(
        out.contains(&format!("\n更新：{NOW}")),
        "timestamp line: {out}"
    );
    // Must NOT be the full report.
    assert!(
        !out.contains("確認："),
        "warning deliver must not render the full report: {out}"
    );
    assert!(
        !out.contains("OIL-TREND:"),
        "warning deliver must not render OIL-TREND: {out}"
    );
    // Prefix must be oilcon's, not chipcon/inflation-con.
    assert!(
        !out.contains("CHIPCON"),
        "must not use chipcon's message prefix: {out}"
    );
    assert!(
        !out.contains("INFLATION-CON"),
        "must not use inflation-con's message prefix: {out}"
    );
    let _ = std::fs::remove_dir_all(&home);
}

#[tokio::test]
async fn record_plus_warning_goes_to_stderr_and_exits_one() {
    // run.py:324–325  print(f"[ERROR: {snapshot.warning}]", file=sys.stderr); sys.exit(1)
    // Opposite of chipcon/inflation-con, which record warned runs as degraded.
    let conn = mem().await;
    let home = tmp();
    let (code, out, err, home) = go(
        &["oilcon", "--mode", "record"],
        Some(JOB),
        &conn,
        &history_fail,
        &latest_fail,
        home,
    )
    .await;
    assert_eq!(code, 1, "record + warning must exit 1");
    assert!(
        err.starts_with("[ERROR: "),
        "stderr must be the ERROR line: {err:?}"
    );
    assert!(
        err.contains("history fetch failed"),
        "warning text must reach stderr: {err}"
    );
    // No markers, no history log, no delivery.
    assert!(
        !out.contains("[skill-status:"),
        "no markers on the record error path: {out}"
    );
    assert!(
        !out.contains("[trace:"),
        "no trace on the record error path: {out}"
    );
    assert!(
        !out.contains("🛢️ OILCON 情報"),
        "record+warning must not deliver: {out}"
    );
    let log = home.join(".nullclaw/oilcon-history.log");
    assert!(
        !log.exists(),
        "a warned record must not write the history log"
    );
    let _ = std::fs::remove_dir_all(&home);
}

// ---------------------------------------------------------------------------
// Record mode (run.py:332–341) — never delivers; hardcoded ok
// ---------------------------------------------------------------------------

#[tokio::test]
async fn record_mode_writes_the_history_line_and_emits_ok_without_delivering() {
    // run.py:332–341  write log, then emit_skill_status("ok"); emit_trace()
    // — bypasses emit_and_exit, so no delivery of the report.
    let (code, out, err, home) =
        go_seeded(&["oilcon", "--mode", "record"], Some(JOB)).await;
    assert_eq!(code, 0);
    assert!(err.is_empty(), "{err}");
    let log = home.join(".nullclaw/oilcon-history.log");
    let text = std::fs::read_to_string(&log).expect("history log not written");
    assert!(
        text.starts_with(NOW_SECS),
        "record line must start with the seconds timestamp: {text}"
    );
    assert!(text.contains(" WTI "), "record line carries WTI: {text}");
    assert!(text.ends_with('\n'), "newline-terminated: {text:?}");
    assert!(
        out.contains("[skill-status:ok]"),
        "record status is hardcoded ok: {out}"
    );
    assert!(out.contains("[trace:job-77]"), "{out}");
    // No deliver: report body must not appear.
    assert!(
        !out.contains("🛢️ OILCON 情報"),
        "record mode must not deliver the report: {out}"
    );
    assert!(
        !out.contains("確認："),
        "record mode must not render the deliver report: {out}"
    );
    // Markers only on stdout (no body before them).
    let status = out.find("[skill-status:ok]").unwrap();
    assert_eq!(
        status, 0,
        "record mode stdout should start with markers, not a delivered body: {out:?}"
    );
    let _ = std::fs::remove_dir_all(&home);
}

#[tokio::test]
async fn record_mode_status_is_hardcoded_ok_independent_of_snapshot() {
    // run.py:340  emit_skill_status("ok") — not derived from classification.
    // A no-uptrend market state is still "ok".
    let (code, out, _, home) =
        go_seeded(&["oilcon", "--mode", "record"], Some(JOB)).await;
    assert_eq!(code, 0);
    assert!(
        out.contains("[skill-status:ok]"),
        "record status is always ok when it records: {out}"
    );
    assert!(
        !out.contains("[skill-status:degraded]"),
        "record must not emit degraded (warnings already exited): {out}"
    );
    let _ = std::fs::remove_dir_all(&home);
}

#[tokio::test]
async fn record_mode_appends_rather_than_truncating() {
    let conn = mem().await;
    seed_all(&conn).await;
    let home = tmp();
    let log = home.join(".nullclaw/oilcon-history.log");
    std::fs::write(&log, "PRIOR LINE\n").unwrap();
    let (code, _, _, home) = go(
        &["oilcon", "--mode", "record"],
        Some(JOB),
        &conn,
        &history_unused,
        &latest_ok,
        home,
    )
    .await;
    assert_eq!(code, 0);
    let text = std::fs::read_to_string(&log).unwrap();
    assert!(
        text.starts_with("PRIOR LINE\n"),
        "history must be appended, not replaced: {text}"
    );
    assert_eq!(text.lines().count(), 2, "{text}");
    let _ = std::fs::remove_dir_all(&home);
}

#[tokio::test]
async fn record_history_write_failure_without_mkdir_exits_one() {
    // run.py:334  open(HISTORY_LOG, "a") with no mkdir.
    // run.py:337–338  [ERROR: could not write history log - {exc}] + exit 1.
    let conn = mem().await;
    seed_all(&conn).await;
    // Home with NO .nullclaw directory — open for append fails.
    let home = std::env::temp_dir().join(format!(
        "oilcon-nomkdir-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    // Deliberately do NOT create home/.nullclaw.
    let (code, out, err, home) = go(
        &["oilcon", "--mode", "record"],
        Some(JOB),
        &conn,
        &history_unused,
        &latest_ok,
        home,
    )
    .await;
    assert_eq!(code, 1, "history write failure must exit 1");
    assert!(
        err.starts_with("[ERROR: could not write history log - "),
        "exact history-log error prefix: {err:?}"
    );
    assert!(
        !out.contains("[skill-status:"),
        "no markers when the write fails: {out}"
    );
    let _ = std::fs::remove_dir_all(&home);
}

// ---------------------------------------------------------------------------
// Inherited from Task 2 — format_message expect / exit-code invariant
// ---------------------------------------------------------------------------

#[tokio::test]
async fn warning_free_deliver_always_has_wti_rows_so_format_message_expect_is_unreachable() {
    // Task 2 recorded: format_message calls format_wti_line(...).expect("WTI rows
    // are required"), which would exit 101 on panic vs Python's exit 1 on
    // ValueError. It is unreachable today because:
    //   1. run short-circuits on snapshot.warning before format_message, and
    //   2. build_symbol_snapshot raises for WTI rather than rows=None, so a
    //      warning-free snapshot always carries WTI rows.
    // Option chosen: **assert the invariant** (leave the expect; pin the property).
    let (code, out, err, home) = go_seeded(&["oilcon"], Some(JOB)).await;
    assert_eq!(code, 0, "success path must exit 0, not panic/101");
    assert!(err.is_empty(), "{err}");
    assert!(
        out.contains("WTI: $"),
        "warning-free deliver always renders WTI prices (rows present): {out}"
    );
    assert!(
        out.contains("[skill-status:ok]"),
        "success path emits ok, not a panic: {out}"
    );
    // The half that actually guards the expect: WTI holding 1..20 rows with a
    // WORKING store. That is the only way to reach the MIN_HISTORY_ROWS check
    // with no warning already set, so it is the only input where "WTI errors"
    // versus "WTI returns rows=None" decides between exit 1 and a panic.
    //
    // Verified by breaking the invariant — replacing snapshot.rs's WTI
    // `return Err(...)` with a `rows: None` fallthrough left every contract test
    // green until this block existed, because the empty-store cases below abort
    // inside after_failed_refresh long before the row-count check.
    {
        let conn = mem().await;
        let nineteen = &adequate_series()[..19];
        for r in nineteen {
            upsert(&conn, "CL=F", &r.day, r.close, "yahoo").await.unwrap();
        }
        for r in &adequate_series() {
            for t in ["BZ=F", "HO=F"] {
                upsert(&conn, t, &r.day, r.close, "yahoo").await.unwrap();
            }
        }
        // History fails, so after_failed_refresh keeps the 19 stored rows and the
        // MIN_HISTORY_ROWS check is reached with the store intact.
        let (code, out, err, _h) =
            go(&["oilcon"], Some(JOB), &conn, &history_fail, &latest_fail, tmp()).await;
        assert_eq!(code, 0, "insufficient WTI must degrade, not panic: {err}");
        assert!(
            out.contains("[WARN: insufficient WTI history (19 rows)]"),
            "insufficient WTI must surface as a warning, never reach format_message: {out}"
        );
        assert!(
            !out.contains("WTI: $"),
            "and must not render the full report: {out}"
        );
    }

    // The remaining half: an empty store with a failing history also becomes a
    // warning (three-line message), never reaches format_message.
    let conn = mem().await; // empty + history fail → warning, not format_message
    let home2 = tmp();
    let (code2, out2, _, home2) = go(
        &["oilcon"],
        Some(JOB),
        &conn,
        &history_fail,
        &latest_fail,
        home2,
    )
    .await;
    assert_eq!(code2, 0);
    assert!(
        out2.contains("[skill-status:degraded]"),
        "WTI failure is the warning path, not a panic: {out2}"
    );
    assert!(
        !out2.contains("確認："),
        "warning path never enters format_message: {out2}"
    );
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_dir_all(&home2);
}

// ── argument validation: argparse's refusals, including the exit code ───────
//
// run.py uses argparse with choices=["deliver","record"], so an unknown flag, an
// invalid --mode value and a --mode with no value all exit 2 before any work.
//
// The port originally ignored unknown flags and accepted any --mode string, which
// was not cosmetic: `if args.mode == "deliver"` is false for a typo, so `--mode
// recrod` fell through to the record branch, which never delivers. The nightly
// signal would have stopped arriving while the run still emitted
// [skill-status:ok] and the scheduler saw success. Found on 2026-07-31 by running
// tools/install-skill.sh's own smoke probe, which requires exit 2 and which the
// manual install had bypassed.

#[tokio::test]
async fn an_unknown_flag_exits_two_without_doing_any_work() {
    let conn = mem().await;
    let called = std::cell::Cell::new(false);
    let history = |s: &str| -> Result<Vec<Row>, FetchError> {
        called.set(true);
        history_unused(s)
    };
    let latest = |s: &str| -> Result<Option<Row>, FetchError> {
        called.set(true);
        latest_ok(s)
    };
    let (code, out, err, home) =
        go(&["oilcon", "--nosuchflag"], Some(JOB), &conn, &history, &latest, tmp()).await;
    assert_eq!(code, 2, "argparse exits 2 on an unrecognized argument");
    assert!(
        err.contains("unrecognized arguments: --nosuchflag"),
        "the refusal must name the offending flag: {err}"
    );
    assert!(out.is_empty(), "nothing is delivered or emitted: {out}");
    assert!(
        !called.get(),
        "argparse refuses before any fetch — a bad argument must not reach build_snapshot"
    );
    let _ = std::fs::remove_dir_all(&home);
}

#[tokio::test]
async fn an_invalid_mode_value_exits_two_rather_than_silently_recording() {
    // The dangerous case: `deliver` mistyped is not `deliver`, and every branch
    // that is not `deliver` records — which never delivers.
    let conn = mem().await;
    let (code, out, err, home) = go(
        &["oilcon", "--mode", "recrod"],
        Some(JOB),
        &conn,
        &history_unused,
        &latest_ok,
        tmp(),
    )
    .await;
    assert_eq!(code, 2, "argparse exits 2 on an invalid choice");
    assert!(
        err.contains("invalid choice: 'recrod'"),
        "the refusal must name the rejected value: {err}"
    );
    assert!(
        !out.contains("[skill-status:"),
        "a rejected run must not report a status at all: {out}"
    );
    let _ = std::fs::remove_dir_all(&home);
}

#[tokio::test]
async fn a_flag_missing_its_value_exits_two() {
    let conn = mem().await;
    let (code, _out, err, home) = go(
        &["oilcon", "--mode"],
        Some(JOB),
        &conn,
        &history_unused,
        &latest_ok,
        tmp(),
    )
    .await;
    assert_eq!(code, 2);
    assert!(
        err.contains("argument --mode: expected one argument"),
        "{err}"
    );
    let _ = std::fs::remove_dir_all(&home);
}

#[tokio::test]
async fn both_valid_modes_are_still_accepted() {
    // The guard must not reject what argparse allows.
    for mode in ["deliver", "record"] {
        let conn = mem().await;
        seed_all(&conn).await;
        let (code, _out, err, home) = go(
            &["oilcon", "--mode", mode],
            Some(JOB),
            &conn,
            &history_unused,
            &latest_ok,
            tmp(),
        )
        .await;
        assert_eq!(code, 0, "--mode {mode} must be accepted: {err}");
        let _ = std::fs::remove_dir_all(&home);
    }
}
