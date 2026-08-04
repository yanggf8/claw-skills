//! Mode dispatch, delivery, and nullclaw marker contract for cds-con.
//!
//! No Python original. The contract is plan decision 7 (status reports the
//! **run**, not the data) plus `tools/install-skill.sh`'s argparse refusals
//! (exit 2 before any store work). Marker shape follows chipcon: bare job id,
//! deliver → emit_skill_status → emit_trace, markers only when a job id is set.
//!
//! Writers / Env / store access / clock are injected so every branch is
//! testable without network, Telegram, or a live Turso.

use std::io::Write;
use std::path::PathBuf;

use claw_core::delivery::{deliver, DeliverOptions, DeliveryOutcome};
use credit_store::{parse_series_list, read_credit_history, Observation};
use libsql::Connection;

use crate::render::{format_message, Frequency, SeriesInput};

/// Injected environment: job id (`NULLCLAW_JOB_ID`) and HOME for paths.
#[derive(Debug, Clone)]
pub struct Env {
    pub job_id: Option<String>,
    pub home: PathBuf,
}

/// How the run reaches the store. `Unreachable` is the connect-failure path
/// from `main`; `Ok` is a live (or in-memory) connection.
pub enum StoreAccess<'a> {
    Ok(&'a Connection),
    Unreachable(String),
}

struct Args {
    mode: String,
    deliver_to: Option<String>,
    account: String,
}

/// Parse argv the way argparse does, **including its refusals**.
///
/// Unknown flag, invalid `--mode`, and a flag missing its value all return
/// `Err` so the caller can exit 2 before any store work. There is no Python
/// original here; the contract is `tools/install-skill.sh`'s smoke probe
/// (requires exit 2) and the Phase ③ lesson of 2026-07-31.
///
/// cds-con is deliver-only: fetch lives in `price cds fetch`. The only valid
/// mode is `deliver` (also the default).
fn parse_args(argv: &[String]) -> Result<Args, String> {
    const MODES: [&str; 1] = ["deliver"];

    let mut mode = "deliver".to_string();
    let mut deliver_to: Option<String> = None;
    let mut account = "main".to_string();

    // Skip program name when present (first element that does not look like a flag).
    let mut args = argv;
    if let Some(first) = argv.first() {
        if !first.starts_with('-') {
            args = &argv[1..];
        }
    }

    // Consume the value belonging to `flag`, or refuse the way argparse does.
    fn value_for(args: &[String], i: &mut usize, flag: &str) -> Result<String, String> {
        *i += 1;
        args.get(*i)
            .cloned()
            .ok_or_else(|| format!("cds-con: error: argument {flag}: expected one argument"))
    }

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--mode" => mode = value_for(args, &mut i, "--mode")?,
            "--deliver-to" => deliver_to = Some(value_for(args, &mut i, "--deliver-to")?),
            "--account" => account = value_for(args, &mut i, "--account")?,
            other => {
                return Err(format!("cds-con: error: unrecognized arguments: {other}"));
            }
        }
        i += 1;
    }

    if !MODES.contains(&mode.as_str()) {
        return Err(format!(
            "cds-con: error: argument --mode: invalid choice: '{mode}' (choose from 'deliver')"
        ));
    }

    Ok(Args {
        mode,
        deliver_to,
        account,
    })
}

/// Emit `[skill-status:<status>]` only when job_id is set (manual runs stay clean).
fn emit_skill_status(status: &str, env: &Env, out: &mut dyn Write) {
    if env.job_id.is_none() {
        return;
    }
    let _ = writeln!(out, "[skill-status:{status}]");
    let _ = out.flush();
}

/// Emit `[trace:<job_id>]` only when job_id is set.
fn emit_trace(env: &Env, out: &mut dyn Write) {
    let Some(ref id) = env.job_id else {
        return;
    };
    let _ = writeln!(out, "[trace:{id}]");
    let _ = out.flush();
}

/// Deliver options for cds-con.
///
/// `claw-core`'s default is `Some("Markdown")`. This message has no Markdown
/// markup and the job id is appended bare (not backticked), so we pin
/// `parse_mode: None` deliberately — same choice as chipcon/inflation-con.
pub fn deliver_options(account: &str) -> DeliverOptions {
    DeliverOptions {
        account: account.to_string(),
        parse_mode: None,
        ..Default::default()
    }
}

/// Deliver then markers. Job id is appended bare (not backticks — oilcon differs).
/// On hard delivery failure returns 1 without markers.
fn emit(
    message: &str,
    status: &str,
    deliver_to: Option<&str>,
    account: &str,
    env: &Env,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let mut output = message.to_string();
    if let Some(ref job_id) = env.job_id {
        output.push_str("\n\n");
        output.push_str(job_id);
    }
    let opts = deliver_options(account);
    // claw-core::deliver takes `impl Write` (Sized). Buffer through Vec so we
    // can still accept `&mut dyn Write` on the run seam.
    let (mut o_buf, mut e_buf) = (Vec::new(), Vec::new());
    let outcome = deliver(deliver_to, &output, &opts, &mut o_buf, &mut e_buf);
    let _ = out.write_all(&o_buf);
    let _ = err.write_all(&e_buf);
    let _ = out.flush();
    let _ = err.flush();
    if outcome == DeliveryOutcome::FailedFatal {
        return 1;
    }
    // Order is load-bearing: deliver → status → trace.
    emit_skill_status(status, env, out);
    emit_trace(env, out);
    0
}

/// Hard-failure path: no delivery. Markers only when a job id is set.
/// Decision 7 / CLAUDE.md option A: when there is no usable result, emit
/// `failed` and send nothing so a retry is the only delivery.
fn finish_failed(message: &str, env: &Env, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let _ = writeln!(err, "CDS-CON failed: {message}");
    let _ = err.flush();
    emit_skill_status("failed", env, out);
    emit_trace(env, out);
    1
}

/// Read `cds_series` from the registry `config` table and load each series'
/// history into [`SeriesInput`]s. Frequency is inferred from observation gaps
/// (median ≥ 20 days → monthly); empty series default to daily.
async fn load_series(conn: &Connection) -> Result<Vec<SeriesInput>, String> {
    let raw = read_config(conn, "cds_series")
        .await?
        .ok_or_else(|| {
            "missing config key 'cds_series' in price-registry; set it with: price config set cds_series <value>"
                .to_string()
        })?;
    let specs = parse_series_list(&raw).map_err(|e| e.to_string())?;
    let mut out = Vec::with_capacity(specs.len());
    for spec in specs {
        let history = read_credit_history(conn, &spec.key)
            .await
            .map_err(|e| e.message().to_string())?;
        let frequency = infer_frequency(&history);
        let rows: Vec<Observation> = history
            .into_iter()
            .map(|(date, value)| Observation { date, value })
            .collect();
        out.push(SeriesInput {
            spec,
            rows,
            frequency,
        });
    }
    Ok(out)
}

/// Days of the month on which the monthly block expands. Data, not code --
/// absent or unparseable key means 7.
async fn monthly_expand_days(conn: &Connection) -> u32 {
    read_config(conn, "cds_monthly_expand_days")
        .await
        .ok()
        .flatten()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(7)
}

async fn read_config(conn: &Connection, key: &str) -> Result<Option<String>, String> {
    let mut rows = conn
        .query(
            "SELECT value FROM config WHERE key = ?",
            libsql::params![key],
        )
        .await
        .map_err(|e| e.to_string())?;
    match rows.next().await.map_err(|e| e.to_string())? {
        Some(r) => Ok(Some(r.get::<String>(0).map_err(|e| e.to_string())?)),
        None => Ok(None),
    }
}

/// Infer publication frequency from consecutive observation gaps.
/// Median gap ≥ 20 calendar days → monthly; otherwise daily. Empty or
/// single-point series default to daily (no gap to measure).
fn infer_frequency(rows: &[(String, f64)]) -> Frequency {
    if rows.len() < 2 {
        return Frequency::Daily;
    }
    let mut gaps: Vec<i64> = Vec::new();
    for w in rows.windows(2).take(32) {
        if let (Some(a), Some(b)) = (parse_ymd(&w[0].0), parse_ymd(&w[1].0)) {
            let g = (date_to_rata_die(b.0, b.1, b.2) - date_to_rata_die(a.0, a.1, a.2)).abs();
            if g > 0 {
                gaps.push(g);
            }
        }
    }
    if gaps.is_empty() {
        return Frequency::Daily;
    }
    gaps.sort_unstable();
    let median = gaps[gaps.len() / 2];
    if median >= 20 {
        Frequency::Monthly
    } else {
        Frequency::Daily
    }
}

fn parse_ymd(s: &str) -> Option<(i32, u32, u32)> {
    if s.len() < 10 {
        return None;
    }
    let y: i32 = s[0..4].parse().ok()?;
    let m: u32 = s[5..7].parse().ok()?;
    let d: u32 = s[8..10].parse().ok()?;
    Some((y, m, d))
}

fn date_to_rata_die(y: i32, m: u32, d: u32) -> i64 {
    let y = y as i64;
    let m = m as i64;
    let d = d as i64;
    let (y, m) = if m <= 2 {
        (y - 1, m + 9)
    } else {
        (y, m - 3)
    };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let doy = (153 * m + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 306
}

/// Core entry: parse, load, render, deliver, markers.
///
/// Returns the process exit code. Does **not** call `process::exit`.
///
/// - `store` — live connection or an unreachable diagnostic
/// - `as_of` — injected YYYY-MM-DD for age-in-days (no wall clock inside run)
pub async fn run(
    argv: &[String],
    env: &Env,
    store: StoreAccess<'_>,
    as_of: &str,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    // argparse refuses before any work: a bad argument must not reach the store.
    let args = match parse_args(argv) {
        Ok(a) => a,
        Err(msg) => {
            let _ = writeln!(err, "{msg}");
            let _ = err.flush();
            return 2;
        }
    };

    // Currently deliver-only; parse_args already rejected other modes.
    let _ = args.mode;

    let conn = match store {
        StoreAccess::Ok(c) => c,
        StoreAccess::Unreachable(msg) => {
            return finish_failed(&msg, env, out, err);
        }
    };

    let series = match load_series(conn).await {
        Ok(s) => s,
        Err(e) => {
            return finish_failed(&e, env, out, err);
        }
    };

    // Decision 7: zero usable observations is a run failure, not an all-n/a ok.
    if !series.iter().any(|s| !s.rows.is_empty()) {
        return finish_failed(
            "no usable observations — empty store or every configured series missing",
            env,
            out,
            err,
        );
    }

    let expand_days = monthly_expand_days(conn).await;

    // Missing kind fails here (RenderError) — failed, no deliver.
    let message = match format_message(&series, as_of, expand_days) {
        Ok(m) => m,
        Err(e) => {
            return finish_failed(&e.message, env, out, err);
        }
    };

    // Data present (however stale; partial missing OK) → ok and deliver.
    emit(
        &message,
        "ok",
        args.deliver_to.as_deref(),
        &args.account,
        env,
        out,
        err,
    )
}
