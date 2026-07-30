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
