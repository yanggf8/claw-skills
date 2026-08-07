//! Binary-level: the reason has to reach stderr, not just exist.
//!
//! `content_reason.rs` proves `content_gap` can explain itself. It cannot prove
//! main prints it, and printing it is the entire defect — on 2026-08-07 the eod
//! alert read `failure=contract_degraded … no stderr` while every predicate in
//! the crate was working exactly as designed. A unit test on the predicate side
//! of that fork stays green through the whole incident (lessons §1, assertions
//! that never see the composition).
//!
//! The stub fails closed: one scripted response, then a 418 that the client
//! treats as terminal, so an unexpected second request surfaces as a failure
//! instead of a quiet pass.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

const EOD_PLACEHOLDER: &str = include_str!("eod_placeholder.json");
const EOD_SCORECARD: &str = include_str!("eod_scorecard.json");

struct Stub {
    port: u16,
    handle: Option<thread::JoinHandle<()>>,
}

impl Stub {
    fn serving(body: String) -> Stub {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().unwrap().port();
        let served = AtomicUsize::new(0);

        let handle = thread::spawn(move || {
            if let Some(Ok(mut stream)) = listener.incoming().next() {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                loop {
                    let mut line = String::new();
                    match reader.read_line(&mut line) {
                        Ok(0) => break,
                        Ok(_) if line == "\r\n" => break,
                        Ok(_) => {}
                        Err(_) => break,
                    }
                }
                let first = served.fetch_add(1, Ordering::SeqCst) == 0;
                let resp = if first {
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    )
                } else {
                    "HTTP/1.1 418 I'm a teapot\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        .to_string()
                };
                let _ = stream.write_all(resp.as_bytes());
                let _ = stream.flush();
            }
        });

        Stub {
            port,
            handle: Some(handle),
        }
    }

    fn base(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}

impl Drop for Stub {
    fn drop(&mut self) {
        let _ = TcpStream::connect(("127.0.0.1", self.port));
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn envelope(data: &str) -> String {
    format!(r#"{{"success":true,"data":{data},"metadata":{{"source":"fresh"}}}}"#)
}

/// Run the real binary against a stubbed route: (stdout, stderr, exit code).
fn run(data: &str, mode: &str) -> (String, String, i32) {
    let stub = Stub::serving(envelope(data));
    let out = Command::new(env!("CARGO_BIN_EXE_cct"))
        .args(["--mode", mode])
        .env("CCT_BASE", stub.base())
        // A developer's real dotenv and config must not reach this run.
        .env("CLAW_ENV", "/dev/null")
        .env("CLAW_CONFIG", "/dev/null")
        // Markers are gated on this, and the status line is half the assertion.
        .env("NULLCLAW_JOB_ID", "test-trace:1")
        .output()
        .expect("run cct");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.code().unwrap_or(-1),
    )
}

#[test]
fn the_eod_placeholder_degrades_and_says_why_on_stderr() {
    let (stdout, stderr, code) = run(EOD_PLACEHOLDER, "eod");
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(
        stdout.contains("[skill-status:degraded]"),
        "stdout: {stdout}"
    );
    assert!(
        stderr.contains("EOD analysis not yet available"),
        "this is the line the 2026-08-07 alert wanted and did not get. stderr: {stderr:?}"
    );
}

#[test]
fn the_reason_names_the_mode_so_the_alert_points_at_one_of_four_jobs() {
    let (_, stderr, _) = run(EOD_PLACEHOLDER, "eod");
    assert!(stderr.contains("eod"), "stderr: {stderr:?}");
}

#[test]
fn the_diagnosis_stays_off_stdout_where_the_message_body_lives() {
    // stdout is delivered text plus the two markers. A warning there becomes
    // part of what the reader receives on Telegram.
    let (stdout, _, _) = run(EOD_PLACEHOLDER, "eod");
    assert!(!stdout.contains("[WARN"), "stdout: {stdout}");
}

#[test]
fn a_real_scorecard_is_ok_and_silent() {
    let (stdout, stderr, code) = run(EOD_SCORECARD, "eod");
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stdout.contains("[skill-status:ok]"), "stdout: {stdout}");
    assert_eq!(stderr, "", "a good payload must not warn");
}

#[test]
fn every_mode_that_degrades_says_why_through_the_whole_binary() {
    // The hole an adversarial review found in the first version of this file:
    // it exercised `eod` only, so suppressing the warning for one of the other
    // three left the entire suite green — `content_reason.rs` never spawns the
    // process, and this file never asked about them. Lessons §1, the same
    // composition gap in a smaller shape.
    for mode in ["pre-market", "intraday", "eod", "weekly"] {
        let (stdout, stderr, code) = run("{}", mode);
        assert_eq!(code, 0, "{mode}: stderr {stderr}");
        assert!(
            stdout.contains("[skill-status:degraded]"),
            "{mode} called an empty payload usable: {stdout}"
        );
        assert!(
            stderr.contains(&format!("CCT {mode} carries no analysis")),
            "{mode} degraded without a reason: {stderr:?}"
        );
    }
}

#[test]
fn a_hostile_upstream_string_cannot_crowd_the_reason_out_of_the_alert() {
    // nullclaw previews the first 200 BYTES of stderr and sends them as the
    // alert. An unbounded quote would fill that with upstream boilerplate, and
    // a cut landing mid-codepoint would hand Telegram invalid UTF-8 — losing
    // the alert this whole change exists to populate. Multi-byte on purpose.
    let flood = "顆".repeat(4000);
    let data = format!(
        r#"{{"type":"end_of_day_summary","daily_summary":{{"symbols_analyzed":0,"key_events":["{flood}"]}}}}"#
    );
    let (_, stderr, code) = run(&data, "eod");
    assert_eq!(code, 0);
    let line = stderr.lines().next().expect("a warning line");
    assert!(
        line.len() <= 200,
        "the warning must survive nullclaw's 200-byte preview whole, got {} bytes",
        line.len()
    );
    assert!(line.ends_with('…'), "truncation should be visible: {line}");
    assert!(
        std::str::from_utf8(&stderr.as_bytes()[..200.min(stderr.len())]).is_ok(),
        "a 200-byte cut must not split a codepoint"
    );
}

#[test]
fn control_characters_from_upstream_do_not_break_the_warning_into_lines() {
    // A `message` carrying newlines would split the warning, and only the first
    // line reads as the reason. NUL is worse. Both are upstream-controlled.
    let data = r#"{"type":"intraday_check","total_symbols":0,"symbols":[],"message":"line one\nline two\u0000tail"}"#;
    let (_, stderr, code) = run(data, "intraday");
    assert_eq!(code, 0);
    assert_eq!(stderr.lines().count(), 1, "stderr: {stderr:?}");
    assert!(stderr.contains("line one line two"), "stderr: {stderr:?}");
    assert!(!stderr.contains('\u{0}'), "stderr: {stderr:?}");
}

#[test]
fn an_unknown_flag_exits_two() {
    // tools/install-skill.sh probes for exactly this before publishing.
    let out = Command::new(env!("CARGO_BIN_EXE_cct"))
        .arg("--definitely-not-a-flag")
        .env("CLAW_ENV", "/dev/null")
        .output()
        .expect("run cct");
    assert_eq!(out.status.code(), Some(2));
}

/// Run the binary against a caller-supplied envelope: (stdout, stderr, exit code).
fn run_envelope(envelope: String, mode: &str) -> (String, String, i32) {
    let stub = Stub::serving(envelope);
    let out = Command::new(env!("CARGO_BIN_EXE_cct"))
        .args(["--mode", mode])
        .env("CCT_BASE", stub.base())
        .env("CLAW_ENV", "/dev/null")
        .env("CLAW_CONFIG", "/dev/null")
        .env("NULLCLAW_JOB_ID", "test-trace:1")
        .output()
        .expect("run cct");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.code().unwrap_or(-1),
    )
}

/// An envelope carrying the worker's provenance fields.
fn envelope_with(data: &str, business_date: &str, has_content: bool) -> String {
    format!(
        r#"{{"success":true,"data":{data},"metadata":{{"business_date":"{business_date}","has_content":{has_content},"source":"d1_fallback"}}}}"#
    )
}

#[test]
fn a_report_carrying_a_business_date_still_renders_and_classifies() {
    // The ET branch is new code between the fetch and the render, so this pins
    // that a report arriving with provenance still comes out the other side.
    let data = r#"{"type":"pre_market_briefing","date":"2020-01-01","is_stale":false,
                   "high_confidence_signals":[{"symbol":"AAPL"}]}"#;
    let (stdout, stderr, code) = run_envelope(envelope_with(data, "2020-01-01", true), "pre-market");
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stdout.contains("[skill-status:degraded]"), "stdout: {stdout}");
    assert!(stderr.contains("2020-01-01"), "the reason names the day: {stderr:?}");
}

#[test]
fn main_takes_its_clock_from_the_rule_and_not_from_one_of_the_two() {
    // The composition gap this file exists for, in its smallest form. The rule
    // lives in `freshness::comparison_today` and is unit-tested against fixed
    // dates — but which clock main *passes on* is invisible to those tests, and
    // invisible end to end too: ET and UTC name the same day for twenty hours
    // out of twenty-four, so a binary test asserting a rendered date would pass
    // whatever main did, almost always. Asserted over the source instead, which
    // discriminates at every hour.
    let main_rs = include_str!("../src/main.rs");
    assert!(
        main_rs.contains("comparison_today(report.business_date.as_deref(), et_today, utc_today)"),
        "main must derive `today` from the rule, not pick a clock itself"
    );
    // Everything after that *line* — splitting mid-line would catch the rule's
    // own arguments and fail on correct code.
    let after_resolve = main_rs
        .split_once("let today = comparison_today")
        .expect("the resolution line")
        .1
        .split_once('\n')
        .expect("a line ending")
        .1;
    for clock in ["utc_today", "et_today"] {
        assert!(
            !after_resolve.contains(clock),
            "nothing downstream of the rule may reach for {clock} directly"
        );
    }
}

#[test]
fn the_worker_saying_it_found_nothing_is_taken_at_its_word() {
    // The end-of-day placeholder, as production serves it: a well-formed report
    // about a day for which no analysis exists. The per-mode predicates had to
    // infer that from the payload's shape; the envelope states it outright, and
    // the reason names the day so the cron alert points somewhere.
    let data = r#"{"type":"end_of_day_summary","date":"2026-08-07",
                   "daily_summary":{"symbols_analyzed":0,"key_events":["Market closed"]}}"#;
    let (stdout, stderr, code) = run_envelope(envelope_with(data, "2026-08-07", false), "eod");
    assert_eq!(code, 0);
    assert!(stdout.contains("[skill-status:degraded]"), "stdout: {stdout}");
    assert!(stderr.contains("2026-08-07"), "the reason must name the day: {stderr:?}");
    assert!(stderr.contains("eod"), "and the mode: {stderr:?}");
}

#[test]
fn a_payload_the_worker_vouches_for_is_still_checked() {
    // `has_content: true` does not silence the predicates. The envelope is
    // trusted for "nothing here", which it knows about its own storage better
    // than any reader can; it is not trusted for "everything here", which it
    // does not. Those predicates are what caught a dead pipeline serving
    // plausible reports for 50 days, and a field that could switch them off
    // would hand that failure a way back in.
    let data = r#"{"type":"end_of_day_summary","daily_summary":{"symbols_analyzed":0}}"#;
    let (stdout, stderr, code) = run_envelope(envelope_with(data, "2026-08-07", true), "eod");
    assert_eq!(code, 0);
    assert!(
        stdout.contains("[skill-status:degraded]"),
        "an empty payload is degraded however the envelope labels it: {stdout}"
    );
    assert!(!stderr.is_empty(), "and it still says why: {stderr:?}");
}

#[test]
fn the_intraday_header_is_stamped_in_market_time() {
    // The header used to read "… 05:57 UTC". Honest, but it asks a reader who
    // thinks in sessions to convert, and it is the one line in the report still
    // quoting a clock that is not the market's. ET is the market's own time.
    //
    // The hour is the discriminator and it never coincides: ET runs four hours
    // behind UTC in EDT and five in EST, so an ET-stamped header can never show
    // the UTC hour. Asserting the exact rendered minute would be flaky at
    // boundaries; asserting the hour differs is deterministic at every instant.
    let data = r#"{"type":"intraday_check","market_status":"open","total_symbols":0,"symbols":[]}"#;
    let (stdout, _, code) = run_envelope(envelope_with(data, "2026-08-07", true), "intraday");
    assert_eq!(code, 0);

    let header = stdout.lines().next().expect("a header");
    assert!(header.ends_with(" ET"), "header must be stamped ET: {header}");
    assert!(!header.contains("UTC"), "and no longer UTC: {header}");

    let utc_hour = jiff::Timestamp::now()
        .in_tz("UTC")
        .expect("UTC")
        .strftime("%H")
        .to_string();
    let rendered_hour = header
        .rsplit(' ')
        .nth(1)
        .and_then(|hm| hm.split(':').next())
        .expect("an hour in the header")
        .to_string();
    assert_ne!(
        rendered_hour, utc_hour,
        "an ET stamp can never show the UTC hour: {header}"
    );
}
