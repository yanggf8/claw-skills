//! inflation-con binary — thin wrapper around `inflation_con::run::run`.
//!
//! Supplies real argv, process environment, a market-fetch FRED fetcher,
//! the real CST clock, and stdout/stderr, then `process::exit`s the code.

use std::io::Write;
use std::path::PathBuf;

use inflation_con::fetch::live_fetch;
use inflation_con::run::{run, Env};

/// CST wall clock matching Python:
/// `datetime.now(timezone(timedelta(hours=8))).strftime("%Y-%m-%d %H:%M:%S CST")`
fn cst_now() -> String {
    let tz = jiff::tz::TimeZone::fixed(jiff::tz::offset(8));
    jiff::Timestamp::now()
        .to_zoned(tz)
        .strftime("%Y-%m-%d %H:%M:%S CST")
        .to_string()
}

fn main() {
    let mut out = std::io::stdout();
    let mut err = std::io::stderr();

    let argv: Vec<String> = std::env::args().collect();
    let env = Env {
        job_id: std::env::var("NULLCLAW_JOB_ID")
            .ok()
            .filter(|v| !v.is_empty()),
        home: std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(".")),
    };

    let code = run(
        &argv,
        &env,
        &live_fetch,
        &cst_now(),
        &mut out,
        &mut err,
    );
    // Ensure buffers flush before exit (especially when redirected).
    let _ = out.flush();
    let _ = err.flush();
    std::process::exit(code);
}
