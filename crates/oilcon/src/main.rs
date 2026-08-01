//! oilcon binary — thin wrapper around `oilcon::run::run`.
//!
//! Supplies real argv, process environment, a price-registry connection,
//! live Yahoo fetchers, the real CST clock, and stdout/stderr, then
//! `process::exit`s the code. Connection failure degrades with a warning
//! the same way Python's `build_snapshot` does on missing credentials /
//! turso errors — mode dispatch still runs (no silent skip).

use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use claw_core::delivery::{deliver, DeliveryOutcome};
use market_fetch::yahoo::{chart_url, parse_yahoo_chart, FetchError};
use oilcon::analysis::Row;
use oilcon::run::{deliver_options, run, Env};
use turso_util::{connect, RegistryConfig, TokenEnvPolicy, TokenTier};

const YAHOO_CHART_BASE: &str = "https://query1.finance.yahoo.com/v8/finance/chart";
const USER_AGENT: &str = "nullclaw/1.0";
const FETCH_TIMEOUT_SECS: u64 = 15;

/// Same registry locator as price-cli (`PRICE_TURSO_URL` / write token).
fn price_registry() -> RegistryConfig {
    RegistryConfig {
        db_name: "price-registry".into(),
        db_name_envs: vec!["PRICE_TURSO_DB".into()],
        db_url_envs: vec!["PRICE_TURSO_URL".into()],
        operator_env: "PRICE_OPERATOR".into(),
        config_home_env: "GWEBCDB_CONFIG_HOME".into(),
        cache_namespace: "gwebcdb".into(),
        token_envs: TokenEnvPolicy {
            read: vec!["PRICE_TURSO_READ_TOKEN".into()],
            write: vec!["PRICE_TURSO_WRITE_TOKEN".into()],
            secrets: vec![],
            allow_generic_fallback: false,
        },
        supported_tiers: vec![TokenTier::Read, TokenTier::Write],
    }
}

fn fetch_chart(sym: &str, range: &str) -> Result<Vec<Row>, FetchError> {
    let url = chart_url(YAHOO_CHART_BASE, sym, range);
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

fn live_fetch_history(sym: &str) -> Result<Vec<Row>, FetchError> {
    fetch_chart(sym, "1y")
}

fn live_fetch_latest(sym: &str) -> Result<Option<Row>, FetchError> {
    let rows = fetch_chart(sym, "5d")?;
    Ok(rows.into_iter().last())
}

/// CST wall clock matching Python `cst_now` / `cst_now(with_seconds=True)`.
fn cst_now(with_seconds: bool) -> String {
    let tz = jiff::tz::TimeZone::fixed(jiff::tz::offset(8));
    let z = jiff::Timestamp::now().to_zoned(tz);
    if with_seconds {
        z.strftime("%Y-%m-%d %H:%M:%S CST").to_string()
    } else {
        z.strftime("%Y-%m-%d %H:%M").to_string()
    }
}

fn cst_today() -> String {
    let tz = jiff::tz::TimeZone::fixed(jiff::tz::offset(8));
    jiff::Timestamp::now()
        .to_zoned(tz)
        .strftime("%Y-%m-%d")
        .to_string()
}

fn parse_mode_and_delivery(argv: &[String]) -> (String, Option<String>, String) {
    let mut mode = "deliver".to_string();
    let mut deliver_to: Option<String> = None;
    let mut account = "main".to_string();

    let mut args = argv;
    if let Some(first) = argv.first() {
        if !first.starts_with('-') {
            args = &argv[1..];
        }
    }
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--mode" => {
                i += 1;
                if i < args.len() {
                    mode = args[i].clone();
                }
            }
            "--deliver-to" => {
                i += 1;
                if i < args.len() {
                    deliver_to = Some(args[i].clone());
                }
            }
            "--account" => {
                i += 1;
                if i < args.len() {
                    account = args[i].clone();
                }
            }
            _ => {}
        }
        i += 1;
    }
    (mode, deliver_to, account)
}

/// Mode dispatch for a pre-built warning when the registry cannot be opened.
/// Mirrors run.py:319–326 without a store (Python's build_snapshot returns
/// Snapshot(warning=…) and main continues into the same branches).
fn dispatch_warning(
    argv: &[String],
    env: &Env,
    warning: &str,
    now: &str,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let (mode, deliver_to, account) = parse_mode_and_delivery(argv);

    if mode == "deliver" {
        let mut output = format!(
            "🛢️ OILCON 情報\n⚠ 今天沒有報告可出:{warning}\n  沒有數字可讀,不是行情平靜。下次排程會再試。\n更新：{now}"
        );
        if let Some(ref job_id) = env.job_id {
            output.push_str("\n\n`");
            output.push_str(job_id);
            output.push('`');
        }
        let opts = deliver_options(&account);
        let (mut o_buf, mut e_buf) = (Vec::new(), Vec::new());
        let outcome = deliver(deliver_to.as_deref(), &output, &opts, &mut o_buf, &mut e_buf);
        let _ = out.write_all(&o_buf);
        let _ = err.write_all(&e_buf);
        if outcome == DeliveryOutcome::FailedFatal {
            return 1;
        }
        if env.job_id.is_some() {
            let _ = writeln!(out, "[skill-status:degraded]");
            if let Some(ref id) = env.job_id {
                let _ = writeln!(out, "[trace:{id}]");
            }
        }
        return 0;
    }

    let _ = writeln!(err, "[ERROR: {warning}]");
    1
}

fn connect_warning(e: &turso_util::Error) -> String {
    // Python: MissingCredentialsError → "turso credentials missing"
    //         other connect errors     → "turso unavailable - {exc}"
    let msg = e.message();
    let no_url = std::env::var("PRICE_TURSO_URL").ok().filter(|v| !v.is_empty()).is_none();
    let no_token = std::env::var("PRICE_TURSO_WRITE_TOKEN")
        .ok()
        .filter(|v| !v.is_empty())
        .is_none();
    if no_url || no_token {
        "turso credentials missing".into()
    } else {
        format!("turso unavailable - {msg}")
    }
}

#[tokio::main]
async fn main() {
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

    let now = cst_now(false);
    let now_with_seconds = cst_now(true);
    let today = cst_today();

    let code = match connect(&price_registry(), TokenTier::Write).await {
        Ok(db) => match db.connect() {
            Ok(conn) => {
                run(
                    &argv,
                    &env,
                    &conn,
                    &live_fetch_history,
                    &live_fetch_latest,
                    &now,
                    &now_with_seconds,
                    &today,
                    &mut out,
                    &mut err,
                )
                .await
            }
            Err(e) => {
                let warning = format!("turso unavailable - {e}");
                dispatch_warning(&argv, &env, &warning, &now, &mut out, &mut err)
            }
        },
        Err(e) => {
            let warning = connect_warning(&e);
            dispatch_warning(&argv, &env, &warning, &now, &mut out, &mut err)
        }
    };

    let _ = out.flush();
    let _ = err.flush();
    std::process::exit(code);
}
