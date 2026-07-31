//! Mode dispatch, delivery, and nullclaw marker contract for oilcon.
//!
//! Line-by-line translation of oilcon/scripts/run.py `emit_and_exit` and
//! `main` (lines 295–345), with writers / Env / fetchers / Connection /
//! clock injected so the contract goldens can assert without network or
//! process env mutation.
//!
//! oilcon's contract differs from chipcon and inflation-con on nine points
//! (backticked job id, Markdown parse_mode default, deliver-before-markers
//! order, record never delivers, warning check before mode dispatch, minimal
//! three-line warning message, record+warning → stderr/exit 1, record status
//! hardcoded "ok", history open without mkdir). Do not copy those ports.

use std::io::Write;
use std::path::{Path, PathBuf};

use claw_core::delivery::{deliver, DeliverOptions, DeliveryOutcome};
use libsql::Connection;
use market_fetch::yahoo::FetchError;

use crate::analysis::Row;
use crate::render::{format_message, format_record_line};
use crate::snapshot::build_snapshot;

/// Injected environment: job id (NULLCLAW_JOB_ID) and HOME for paths.
#[derive(Debug, Clone)]
pub struct Env {
    pub job_id: Option<String>,
    pub home: PathBuf,
}

struct Args {
    mode: String,
    deliver_to: Option<String>,
    account: String,
}

/// History log path: `~/.nullclaw/oilcon-history.log` (run.py:18).
fn history_log_path(home: &Path) -> PathBuf {
    home.join(".nullclaw").join("oilcon-history.log")
}

/// Parse argv. `argv[0]` is the program name (tests pass `"oilcon"`).
/// run.py:307–316
/// Parse argv the way `run.py`'s `argparse` does, **including its refusals**.
///
/// An earlier version silently ignored unknown flags and accepted any `--mode`
/// value. That is not a cosmetic difference from the Python: `if args.mode ==
/// "deliver"` is false for a typo, so `--mode recrod` fell through to the record
/// branch, which never delivers — the nightly signal would stop arriving while
/// the run still reported `[skill-status:ok]` and the scheduler saw success.
/// `argparse` exits 2 on all three of these; so do we.
///
/// The exit code is the contract. The message text is not byte-comparable with
/// `argparse`'s usage block and is not attempted — same class as the io::Error
/// versus OSError difference in the history-log line.
fn parse_args(argv: &[String]) -> Result<Args, String> {
    const MODES: [&str; 2] = ["deliver", "record"];

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
            .ok_or_else(|| format!("oilcon: error: argument {flag}: expected one argument"))
    }

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--mode" => mode = value_for(args, &mut i, "--mode")?,
            "--deliver-to" => deliver_to = Some(value_for(args, &mut i, "--deliver-to")?),
            "--account" => account = value_for(args, &mut i, "--account")?,
            other => {
                return Err(format!("oilcon: error: unrecognized arguments: {other}"));
            }
        }
        i += 1;
    }

    if !MODES.contains(&mode.as_str()) {
        return Err(format!(
            "oilcon: error: argument --mode: invalid choice: '{mode}' (choose from 'deliver', 'record')"
        ));
    }

    Ok(Args {
        mode,
        deliver_to,
        account,
    })
}

/// Emit `[skill-status:<status>]` only when job_id is set (manual runs stay clean).
/// lib/trace_marker.py:17–27
fn emit_skill_status(status: &str, env: &Env, out: &mut dyn Write) {
    if env.job_id.is_none() {
        return;
    }
    let _ = writeln!(out, "[skill-status:{status}]");
    let _ = out.flush();
}

/// Emit `[trace:<job_id>]` only when job_id is set.
/// lib/trace_marker.py:30–34
fn emit_trace(env: &Env, out: &mut dyn Write) {
    let Some(ref id) = env.job_id else {
        return;
    };
    let _ = writeln!(out, "[trace:{id}]");
    let _ = out.flush();
}

/// Deliver options for oilcon. `parse_mode` stays at its default (`Some("Markdown")`)
/// — oilcon never overrides it (lib/delivery.py:24). The backticks on the job id
/// are intentional Markdown. chipcon/inflation-con set `parse_mode: None`.
pub fn deliver_options(account: &str) -> DeliverOptions {
    DeliverOptions {
        account: account.to_string(),
        ..Default::default()
    }
}

/// Deliver then markers. Job id is wrapped in backticks (oilcon differs from
/// chipcon/inflation-con which append bare). Order is load-bearing:
/// `deliver_or_fail` → `emit_skill_status` → `emit_trace` (run.py:295–303).
/// On hard delivery failure returns 1 without markers (Python deliver_or_fail exits).
fn emit_and_exit(
    message: &str,
    status: &str,
    deliver_to: Option<&str>,
    account: &str,
    env: &Env,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    // run.py:296–299  if job_id: output += f"\n\n`{job_id}`"
    let mut output = message.to_string();
    if let Some(ref job_id) = env.job_id {
        output.push_str("\n\n`");
        output.push_str(job_id);
        output.push('`');
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
        // Python deliver_or_fail sys.exit(1) before markers.
        return 1;
    }
    // run.py:302–303 — markers AFTER delivery.
    emit_skill_status(status, env, out);
    emit_trace(env, out);
    0
}

/// Core entry: parse, build_snapshot, warning check before mode dispatch,
/// deliver vs record branches. Returns the process exit code. Does **not**
/// call `process::exit`.
///
/// Async because `build_snapshot` / the store are async.
///
/// - `now` — `cst_now()` without seconds, for the deliver message (`更新：…`)
/// - `now_with_seconds` — `cst_now(with_seconds=True)`, for the record line
/// - `today` — calendar day for `needs_backfill` (same zone boundary as `now`)
pub async fn run(
    argv: &[String],
    env: &Env,
    conn: &Connection,
    fetch_history: &dyn Fn(&str) -> Result<Vec<Row>, FetchError>,
    fetch_latest: &dyn Fn(&str) -> Result<Option<Row>, FetchError>,
    now: &str,
    now_with_seconds: &str,
    today: &str,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    // argparse refuses before doing any work, and so must this: a bad argument
    // must not reach build_snapshot, which fetches and writes the registry.
    let args = match parse_args(argv) {
        Ok(a) => a,
        Err(msg) => {
            let _ = writeln!(err, "{msg}");
            let _ = err.flush();
            return 2;
        }
    };

    let snapshot = build_snapshot(conn, today, fetch_history, fetch_latest).await;

    // run.py:319–326 — warning check BEFORE mode dispatch.
    if let Some(ref warning) = snapshot.warning {
        if args.mode == "deliver" {
            // run.py:321  three-line minimal message, not the full report.
            // The whole report is gone, not just a part of it — say so plainly.
            // A reader seeing three lines needs to know this is not "oil is quiet".
            let message = format!(
                "🛢️ OILCON 情報\n⚠ 今天沒有報告可出:{warning}\n  沒有數字可讀,不是行情平靜。下次排程會再試。\n更新：{now}"
            );
            return emit_and_exit(
                &message,
                "degraded",
                args.deliver_to.as_deref(),
                &args.account,
                env,
                out,
                err,
            );
        }
        // run.py:324–325  record + warning → stderr and exit 1 (no markers).
        let _ = writeln!(err, "[ERROR: {warning}]");
        let _ = err.flush();
        return 1;
    }

    if args.mode == "deliver" {
        // run.py:328–330
        // format_message's expect("WTI rows are required") is unreachable here:
        // we short-circuited on warning, and build_symbol_snapshot raises for WTI
        // rather than returning rows=None. See contract test pinning the invariant.
        let (message, status) = format_message(&snapshot, now);
        return emit_and_exit(
            &message,
            &status,
            args.deliver_to.as_deref(),
            &args.account,
            env,
            out,
            err,
        );
    }

    // run.py:332–341  record mode — never delivers; bypasses emit_and_exit.
    // Status is hardcoded "ok" (a warning already exited above).
    match write_history_line(env, &snapshot, now_with_seconds) {
        Ok(()) => {
            emit_skill_status("ok", env, out);
            emit_trace(env, out);
            0
        }
        Err(e) => {
            // run.py:337–338
            let _ = writeln!(err, "[ERROR: could not write history log - {e}]");
            let _ = err.flush();
            1
        }
    }
}

/// Append one history line. No mkdir — plain open for append (run.py:334–335).
/// Format refusals and I/O errors both surface as Err for the caller to map
/// to the history-log error line (run.py:332–338).
fn write_history_line(
    env: &Env,
    snapshot: &crate::snapshot::Snapshot,
    now_with_seconds: &str,
) -> Result<(), String> {
    let line = format_record_line(snapshot, now_with_seconds).map_err(|e| e.to_string())?;
    let path = history_log_path(&env.home);
    // No create_dir_all — parent must already exist. oilcon differs from
    // chipcon/inflation-con which mkdir first.
    use std::fs::OpenOptions;
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| e.to_string())?;
    f.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
    f.write_all(b"\n").map_err(|e| e.to_string())?;
    Ok(())
}
