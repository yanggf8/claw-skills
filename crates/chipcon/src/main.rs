//! chipcon binary — thin wrapper around `chipcon::run::run`.
//!
//! Supplies real argv, process environment, a market-fetch Yahoo fetcher,
//! the real CST clock, and stdout/stderr, then `process::exit`s the code.

use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use chipcon::analysis::Row;
use chipcon::run::{run, Env};
use market_fetch::yahoo::{chart_url, parse_yahoo_chart, FetchError};

const YAHOO_CHART_BASE: &str = "https://query1.finance.yahoo.com/v8/finance/chart";
const USER_AGENT: &str = "nullclaw/1.0";
const FETCH_TIMEOUT_SECS: u64 = 15;

fn live_fetch(sym: &str) -> Result<Vec<Row>, FetchError> {
    let url = chart_url(YAHOO_CHART_BASE, sym, "1y");
    let body = claw_core::http::agent(Duration::from_secs(FETCH_TIMEOUT_SECS))
        .get(&url)
        .set("User-Agent", USER_AGENT)
        .set("Accept", "application/json")
        .call()
        .map_err(|e| FetchError::Http(e.to_string()))?
        .into_string()
        .map_err(|e| FetchError::Http(e.to_string()))?;
    let quotes = parse_yahoo_chart(&body)?;
    Ok(quotes
        .into_iter()
        .map(|q| Row {
            day: q.date,
            close: q.close,
        })
        .collect())
}

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
