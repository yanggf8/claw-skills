//! Nullclaw marker + mode-dispatch goldens for inflation-con.
//!
//! Expectations are taken from inflation-con/scripts/run.py
//! `parse_args` (68-74), `emit` (300-311), and `main` (314-335), and from
//! lib/trace_marker.py (job-id gate) / lib/delivery.py (None → stdout).
//! Shape adapted from crates/chipcon/tests/contract.rs — every string is
//! inflation-con's, not chipcon's.

use inflation_con::analysis::Obs;
use inflation_con::run::{run, Env};
use market_fetch::fred::CreditError;

/// Enough monthly history for classify to leave INSUFFICIENT_DATA.
fn rows(n: usize) -> Vec<Obs> {
    (0..n)
        .map(|i| Obs {
            day: format!("2026-{:02}-01", i % 12 + 1),
            value: 100.0 + i as f64,
        })
        .collect()
}

/// Every series succeeds with enough history to classify.
fn good(_sid: &str) -> Result<Vec<Obs>, CreditError> {
    Ok(rows(12))
}

fn env(job: Option<&str>, home: &std::path::Path) -> Env {
    Env {
        job_id: job.map(String::from),
        home: home.to_path_buf(),
    }
}

fn tmp() -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!(
        "inflation-con-c-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn go(
    argv: &[&str],
    job: Option<&str>,
    f: &dyn Fn(&str) -> Result<Vec<Obs>, CreditError>,
) -> (i32, String, String, std::path::PathBuf) {
    let home = tmp();
    let a: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    let (mut o, mut e) = (Vec::new(), Vec::new());
    let code = run(
        &a,
        &env(job, &home),
        f,
        "2026-07-29 05:30:00 CST",
        &mut o,
        &mut e,
    );
    (
        code,
        String::from_utf8(o).unwrap(),
        String::from_utf8(e).unwrap(),
        home,
    )
}

/// run.py:300-311 emit: deliver body first, then skill-status, then trace.
/// run.py:254 format_message starts with "📈 INFLATION-CON".
#[test]
fn markers_come_after_the_body_and_in_status_then_trace_order() {
    let (code, out, err, _) = go(&["inflation-con"], Some("job-77"), &good);
    assert_eq!(code, 0);
    assert!(err.is_empty(), "stderr must be quiet on success: {err}");
    // run.py:254  lines = ["📈 INFLATION-CON"]
    let body = out.find("INFLATION-CON").expect("body missing");
    let status = out.find("[skill-status:ok]").expect("status marker missing");
    let trace = out.find("[trace:job-77]").expect("trace marker missing");
    assert!(body < status, "body must precede the markers");
    assert!(status < trace, "skill-status must precede trace");
}

/// run.py:306-309  if job_id: output += f"\n\n{job_id}"  — bare, not backticks.
#[test]
fn the_job_id_is_appended_unquoted() {
    let (_, out, _, _) = go(&["inflation-con"], Some("job-77"), &good);
    assert!(
        out.contains("\n\njob-77"),
        "job id must be appended bare: {out}"
    );
    assert!(
        !out.contains("`job-77`"),
        "inflation-con must not quote the job id: {out}"
    );
}

/// lib/trace_marker.py:26,33 — both emits no-op when NULLCLAW_JOB_ID is unset.
#[test]
fn no_job_id_means_no_markers_at_all() {
    let (code, out, _, _) = go(&["inflation-con"], None, &good);
    assert_eq!(code, 0);
    // run.py:254  report still prints
    assert!(out.contains("INFLATION-CON"), "the report itself still prints");
    assert!(
        !out.contains("[skill-status:"),
        "no status marker without a job id: {out}"
    );
    assert!(!out.contains("[trace:"), "no trace marker without a job id: {out}");
}

/// run.py:232-237 secondary failure → warning; format_message → degraded;
/// main returns 0 (run.py:330). Warning reaches the reader via [WARN: …].
#[test]
fn a_secondary_failure_is_degraded_but_still_delivers_and_exits_zero() {
    let f = |sid: &str| -> Result<Vec<Obs>, CreditError> {
        if sid == "PCEPILFE" {
            Ok(rows(12))
        } else {
            Err(CreditError::Http("boom".into()))
        }
    };
    let (code, out, err, _) = go(&["inflation-con"], Some("job-77"), &f);
    assert_eq!(code, 0, "a degraded run is not a failure");
    assert!(out.contains("[skill-status:degraded]"), "{out}");
    // run.py:257  f"[WARN: {warning}]"
    assert!(out.contains("[WARN:"), "the warning must reach the reader: {out}");
    assert!(err.is_empty(), "{err}");
}

/// run.py:331-335  print to stderr, emit failed + trace, return 1.
/// Prefix is "INFLATION-CON failed: " (run.py:332) — not chipcon's.
#[test]
fn a_primary_failure_writes_stderr_emits_failed_and_exits_one() {
    let f = |_: &str| -> Result<Vec<Obs>, CreditError> {
        Err(CreditError::Http("boom".into()))
    };
    let (code, out, err, _) = go(&["inflation-con"], Some("job-77"), &f);
    assert_eq!(code, 1, "a hard failure must exit non-zero");
    // run.py:332  f"INFLATION-CON failed: {exc}"
    assert!(
        err.starts_with("INFLATION-CON failed: "),
        "stderr prefix: {err:?}"
    );
    assert!(out.contains("[skill-status:failed]"), "{out}");
    assert!(out.contains("[trace:job-77]"), "{out}");
}

/// run.py:320-327 record mode: write history line, markers, return 0.
/// No format_message / no report body. History path run.py:321.
#[test]
fn record_mode_writes_the_history_line_before_the_markers() {
    let (code, out, _, home) = go(
        &["inflation-con", "--mode", "record"],
        Some("job-77"),
        &good,
    );
    assert_eq!(code, 0);
    // run.py:321  ~/.nullclaw/inflation-con-history.log
    let log = home.join(".nullclaw/inflation-con-history.log");
    let text = std::fs::read_to_string(&log).expect("history log not written");
    // run.py:291-296  f"{now} INFLATION-CON {status} …"
    assert!(
        text.starts_with("2026-07-29 05:30:00 CST INFLATION-CON "),
        "{text}"
    );
    assert!(
        text.ends_with('\n'),
        "the line must be newline-terminated: {text:?}"
    );
    assert!(
        out.contains("[skill-status:ok]") && out.contains("[trace:job-77]"),
        "{out}"
    );
    // record mode must not render the deliver report (run.py:254 prefix line)
    assert!(
        !out.contains("📈 INFLATION-CON"),
        "record mode must not render the report: {out}"
    );
    let _ = std::fs::remove_dir_all(&home);
}

/// run.py:325  emit_skill_status("degraded" if warning else "ok")
/// inflation-con ACCEPTS a warned record (unlike oilcon).
#[test]
fn record_mode_accepts_a_warned_run_and_reports_degraded() {
    let f = |sid: &str| -> Result<Vec<Obs>, CreditError> {
        if sid == "PCEPILFE" {
            Ok(rows(12))
        } else {
            Ok(vec![])
        }
    };
    let (code, out, _, home) = go(
        &["inflation-con", "--mode", "record"],
        Some("job-77"),
        &f,
    );
    assert_eq!(code, 0);
    let log = home.join(".nullclaw/inflation-con-history.log");
    let text = std::fs::read_to_string(&log).expect("a warned run must still be recorded");
    // run.py:231  f"{series_id}: no rows" — first secondary in DEFAULT_SERIES is CPILFESL
    assert!(text.contains("warning=CPILFESL: no rows"), "{text}");
    assert!(out.contains("[skill-status:degraded]"), "{out}");
    let _ = std::fs::remove_dir_all(&home);
}

/// run.py:323  path.open("a", …) — append, never truncate.
#[test]
fn record_mode_appends_rather_than_truncating() {
    let home = tmp();
    let log = home.join(".nullclaw/inflation-con-history.log");
    std::fs::create_dir_all(log.parent().unwrap()).unwrap();
    std::fs::write(&log, "PRIOR LINE\n").unwrap();
    let a: Vec<String> = ["inflation-con", "--mode", "record"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let (mut o, mut e) = (Vec::new(), Vec::new());
    run(
        &a,
        &env(Some("job-77"), &home),
        &good,
        "2026-07-29 05:30:00 CST",
        &mut o,
        &mut e,
    );
    let text = std::fs::read_to_string(&log).unwrap();
    assert!(
        text.starts_with("PRIOR LINE\n"),
        "history must be appended, not replaced: {text}"
    );
    assert_eq!(text.lines().count(), 2, "{text}");
    let _ = std::fs::remove_dir_all(&home);
}

/// lib/delivery.py:48-50  chat_id empty/None → print body to stdout.
/// run.py:282-283 trailers still present.
#[test]
fn without_deliver_to_the_body_still_reaches_stdout() {
    let (code, out, _, _) = go(&["inflation-con"], Some("job-77"), &good);
    assert_eq!(code, 0);
    assert!(
        out.contains("SIGNAL-ONLY"),
        "the full report must be on stdout: {out}"
    );
    // run.py:283 — second trailer, unique to inflation-con
    assert!(
        out.contains("RED = 進入 review"),
        "inflation-con trailer must be on stdout: {out}"
    );
}
