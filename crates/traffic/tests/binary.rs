//! Binary-level tests: run the real executable end to end.
//!
//! Why this file exists. The unit tests in parity.rs call `minutes_from_seconds`,
//! `body` and `resolve` directly and never go through `main`. An adversarial
//! review found the hole by proposing a breakage the whole suite missed:
//!
//!     // in main.rs only
//!     let minutes = seconds / 60;   // truncating, bypasses the helper
//!
//! 90 seconds then renders as 1 minute instead of 2, on every live route, and
//! all 20 unit tests stay green because none of them is downstream of that
//! line. That is lessons §1, "assertions that never see the composition".
//!
//! These tests spawn the built binary against a local stub. The stub FAILS
//! CLOSED — an unscripted request gets a 418, which the client treats as
//! terminal — so "the binary asked for something we did not expect" shows up
//! as a failure rather than quietly passing (lessons §1 again).

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

/// One-shot HTTP stub. Serves `body` for exactly one request, then 418s.
struct Stub {
    port: u16,
    handle: Option<thread::JoinHandle<()>>,
}

impl Stub {
    fn serving(body: &'static str) -> Stub {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().unwrap().port();
        let served = AtomicUsize::new(0);

        let handle = thread::spawn(move || {
            // One connection is all any of these tests needs, so take exactly
            // one rather than looping with an unconditional break.
            if let Some(Ok(mut stream)) = listener.incoming().next() {
                // Drain the request line and headers so the client is not left
                // writing into a closed socket.
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
                    // Fail closed. 418 is used by nothing here, so a second
                    // request is unmistakably a test failure rather than a
                    // silently-served success.
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
        // Unblock the accept loop if the binary never connected.
        let _ = TcpStream::connect(("127.0.0.1", self.port));
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_traffic")
}

/// Run the binary with a stub route response, returning (stdout, exit code).
fn run_against(seconds_json: &'static str, args: &[&str]) -> (String, i32) {
    let stub = Stub::serving(seconds_json);
    let out = Command::new(bin())
        .args(args)
        .env("TOMTOM_BASE_URL", stub.base())
        .env("TOMTOM_API_KEY", "test-key-not-real")
        // Point the env loader at nothing so a developer's real ~/.nullclaw/.env
        // cannot change the result.
        .env("CLAW_ENV", "/dev/null")
        // No agent call: an empty binary path makes call_agent fail fast, which
        // drops the advice line. Keeps the assertion about minutes only.
        .env("HOME", "/nonexistent-for-traffic-tests")
        .output()
        .expect("run traffic");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        out.status.code().unwrap_or(-1),
    )
}

#[test]
fn ninety_seconds_renders_as_two_minutes_through_the_whole_binary() {
    // The breakage this file exists for. `seconds / 60` gives 1 here;
    // rounding gives 2. Only an end-to-end run can tell the difference.
    let (stdout, code) = run_against(
        r#"{"routes":[{"summary":{"travelTimeInSeconds":90}}]}"#,
        &["--from", "25.1,121.5", "--to", "25.2,121.6"],
    );
    assert_eq!(code, 0, "stdout was: {stdout}");
    assert!(
        stdout.contains("：2分鐘"),
        "expected 2 minutes end to end, got: {stdout}"
    );
}

#[test]
fn an_exact_half_minute_rounds_up_through_the_whole_binary() {
    // 150s → 3. Truncation would give 2 and pass the 90s case only by luck, so
    // both cases are needed to pin the wire-up.
    let (stdout, code) = run_against(
        r#"{"routes":[{"summary":{"travelTimeInSeconds":150}}]}"#,
        &["--from", "25.1,121.5", "--to", "25.2,121.6"],
    );
    assert_eq!(code, 0, "stdout was: {stdout}");
    assert!(
        stdout.contains("：3分鐘"),
        "expected 3 minutes end to end, got: {stdout}"
    );
}

#[test]
fn the_route_line_is_assembled_from_the_names_given() {
    let (stdout, code) = run_against(
        r#"{"routes":[{"summary":{"travelTimeInSeconds":1500}}]}"#,
        &["--from", "25.1,121.5", "--via", "25.15,121.55", "--to", "25.2,121.6"],
    );
    assert_eq!(code, 0, "stdout was: {stdout}");
    assert!(
        stdout.contains("🚗 25.1,121.5→25.15,121.55→25.2,121.6：25分鐘"),
        "got: {stdout}"
    );
}

#[test]
fn a_missing_api_key_warns_and_exits_zero() {
    let out = Command::new(bin())
        .args(["--from", "25.1,121.5", "--to", "25.2,121.6"])
        .env("CLAW_ENV", "/dev/null")
        .env_remove("TOMTOM_API_KEY")
        .output()
        .expect("run traffic");
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "[WARN: traffic unavailable - TOMTOM_API_KEY not set]"
    );
}

#[test]
fn an_unknown_flag_exits_two() {
    // The installer's smoke probe depends on exactly this, and on the binary
    // not performing a real invocation to discover it.
    let out = Command::new(bin())
        .arg("--definitely-not-a-flag")
        .env("CLAW_ENV", "/dev/null")
        .output()
        .expect("run traffic");
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn an_upstream_error_never_puts_the_api_key_on_stdout() {
    // The stub's second response is a 418, so pointing the binary at an
    // already-consumed stub exercises the HTTP error path without needing a
    // real 401. What must not appear is the query string.
    let stub = Stub::serving(r#"{"routes":[]}"#);
    let _ = ureq::get(&format!("{}/warmup", stub.base())).call(); // consume the one good response
    let out = Command::new(bin())
        .args(["--from", "25.1,121.5", "--to", "25.2,121.6"])
        .env("TOMTOM_BASE_URL", stub.base())
        .env("TOMTOM_API_KEY", "SECRET-KEY-VALUE")
        .env("CLAW_ENV", "/dev/null")
        .output()
        .expect("run traffic");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("SECRET-KEY-VALUE"),
        "the API key reached stdout: {stdout}"
    );
    assert!(!stdout.contains("key="), "the query string reached stdout: {stdout}");
}

// ── scheduler markers ────────────────────────────────────────────────────────
//
// These replace commute. The Python arrangement had commute emit the markers
// and traffic omit them; that only held because commute stripped the job id out
// of the child environment. Emitting here is what lets commute be deleted.

#[test]
fn a_scheduled_success_reports_ok_with_the_job_id() {
    let stub = Stub::serving(r#"{"routes":[{"summary":{"travelTimeInSeconds":1500}}]}"#);
    let out = Command::new(bin())
        .args(["--from", "25.1,121.5", "--to", "25.2,121.6"])
        .env("TOMTOM_BASE_URL", stub.base())
        .env("TOMTOM_API_KEY", "test-key-not-real")
        .env("CLAW_ENV", "/dev/null")
        .env("HOME", "/nonexistent-for-traffic-tests")
        .env("NULLCLAW_JOB_ID", "job-abc-123")
        .output()
        .expect("run traffic");
    let stdout = String::from_utf8_lossy(&out.stdout);
    // cron.zig matches these literally, so they are asserted literally.
    assert!(stdout.contains("[skill-status:ok]"), "got: {stdout}");
    assert!(stdout.contains("[trace:job-abc-123]"), "got: {stdout}");
}

#[test]
fn a_scheduled_failure_reports_degraded_not_failed() {
    // degraded, because the run delivered something and a retry returns the
    // same answer. `failed` would have the scheduler retry into an identical
    // result and, on a job with retry_once, deliver twice.
    let out = Command::new(bin())
        .args(["--from", "25.1,121.5", "--to", "25.2,121.6"])
        .env("CLAW_ENV", "/dev/null")
        .env_remove("TOMTOM_API_KEY")
        .env("NULLCLAW_JOB_ID", "job-abc-123")
        .output()
        .expect("run traffic");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0));
    assert!(stdout.contains("[skill-status:degraded]"), "got: {stdout}");
    assert!(stdout.contains("[trace:job-abc-123]"), "got: {stdout}");
}

#[test]
fn a_manual_run_emits_no_markers_at_all() {
    // NULLCLAW_JOB_ID unset. Marker lines in an interactive run would be noise,
    // and a stray [trace:] with no job id is what cron.zig classifies as
    // content_invalid.
    let stub = Stub::serving(r#"{"routes":[{"summary":{"travelTimeInSeconds":1500}}]}"#);
    let out = Command::new(bin())
        .args(["--from", "25.1,121.5", "--to", "25.2,121.6"])
        .env("TOMTOM_BASE_URL", stub.base())
        .env("TOMTOM_API_KEY", "test-key-not-real")
        .env("CLAW_ENV", "/dev/null")
        .env("HOME", "/nonexistent-for-traffic-tests")
        .env_remove("NULLCLAW_JOB_ID")
        .output()
        .expect("run traffic");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(!stdout.contains("[skill-status:"), "got: {stdout}");
    assert!(!stdout.contains("[trace:"), "got: {stdout}");
}
