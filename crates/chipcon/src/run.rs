//! Mode dispatch, delivery, and nullclaw marker contract.
//!
//! Line-by-line translation of chipcon/scripts/run.py `parse_args`, `emit`,
//! and `main`, with writers / Env / fetch / clock injected so the contract
//! goldens can assert without network or process env mutation.
//!
//! Markers are gated on `env.job_id` (the Env seam for NULLCLAW_JOB_ID),
//! matching lib/trace_marker.py: both emit_skill_status and emit_trace are
//! no-ops when the job id is unset. claw-core's marker helpers read the
//! process environment and cannot be used through this seam without races
//! under parallel tests.

use std::io::Write;
use std::path::{Path, PathBuf};

use claw_core::delivery::{deliver, DeliverOptions, DeliveryOutcome};
use market_fetch::yahoo::FetchError;

use crate::analysis::{classify, Row};
use crate::config::load_config;
use crate::fetch::update_state;
use crate::render::{format_message, record_line};

/// Injected environment: job id (NULLCLAW_JOB_ID) and HOME for paths.
#[derive(Debug, Clone)]
pub struct Env {
    pub job_id: Option<String>,
    pub home: PathBuf,
}

struct Args {
    mode: String,
    config: PathBuf,
    deliver_to: Option<String>,
    account: String,
}

/// Parse argv. `argv[0]` is the program name (tests pass `"chipcon"`).
/// Parse argv the way `run.py`'s `argparse` does, **including its refusals**.
///
/// The original port silently ignored unknown flags and accepted any `--mode`
/// value. Not cosmetic: a mistyped `--deliver-to` leaves `deliver_to` at `None`,
/// so the message goes to stdout instead of Telegram and the morning signal
/// stops arriving while the run still reports `[skill-status:ok]`. Same class of
/// defect as the one `tools/install-skill.sh`'s smoke probe caught in oilcon on
/// 2026-07-31; that probe requires exit 2, which argparse gives and this did not.
///
/// The exit code is the contract. The message text is not byte-comparable with
/// argparse's usage block and is not attempted.
fn parse_args(argv: &[String], home: &Path) -> Result<Args, String> {
    const MODES: [&str; 2] = ["deliver", "record"];

    let mut mode = "deliver".to_string();
    let mut config = home
        .join(".nullclaw")
        .join("skills")
        .join("chipcon")
        .join("config.json");
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
            .ok_or_else(|| format!("chipcon: error: argument {flag}: expected one argument"))
    }

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--mode" => mode = value_for(args, &mut i, "--mode")?,
            "--config" => {
                let raw = value_for(args, &mut i, "--config")?;
                // Python: Path(args.config).expanduser()
                config = if let Some(stripped) = raw.strip_prefix("~/") {
                    home.join(stripped)
                } else if raw == "~" {
                    home.to_path_buf()
                } else {
                    PathBuf::from(&raw)
                };
            }
            "--deliver-to" => deliver_to = Some(value_for(args, &mut i, "--deliver-to")?),
            "--account" => {
                account = value_for(args, &mut i, "--account")?
            }
            other => {
                return Err(format!("chipcon: error: unrecognized arguments: {other}"));
            }
        }
        i += 1;
    }

    if !MODES.contains(&mode.as_str()) {
        return Err(format!(
            "chipcon: error: argument --mode: invalid choice: '{mode}' (choose from 'deliver', 'record')"
        ));
    }

    Ok(Args {
        mode,
        config,
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

/// Deliver then markers. Job id is appended bare (not backticks — oilcon differs).
/// On hard delivery failure returns 1 without markers (Python deliver_or_fail exits).
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
    let opts = DeliverOptions {
        account: account.to_string(),
        parse_mode: None,
        ..Default::default()
    };
    // claw-core::deliver takes `impl Write` (Sized). Buffer through Vec so we
    // can still accept `&mut dyn Write` on the run seam without changing the
    // golden signature.
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
    emit_skill_status(status, env, out);
    emit_trace(env, out);
    0
}

/// Core entry: parse, load_config (outside try — wart 1), fetch/classify, mode dispatch.
///
/// Returns the process exit code. Does not call `process::exit`.
pub fn run(
    argv: &[String],
    env: &Env,
    fetch: &dyn Fn(&str) -> Result<Vec<Row>, FetchError>,
    now: &str,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    // argparse refuses before doing any work, and so must this: a bad argument
    // must not reach the fetch.
    let args = match parse_args(argv, &env.home) {
        Ok(a) => a,
        Err(msg) => {
            let _ = writeln!(err, "{msg}");
            let _ = err.flush();
            return 2;
        }
    };
    // load_config runs BEFORE the try — malformed config panics, no markers
    // (wart 1 preserved from run.py).
    let cfg = load_config(&args.config);

    match run_body(&args, env, fetch, now, out, err, &cfg) {
        Ok(code) => code,
        Err(e) => {
            let _ = writeln!(err, "CHIPCON failed: {e}");
            let _ = err.flush();
            emit_skill_status("failed", env, out);
            emit_trace(env, out);
            1
        }
    }
}

fn run_body(
    args: &Args,
    env: &Env,
    fetch: &dyn Fn(&str) -> Result<Vec<Row>, FetchError>,
    now: &str,
    out: &mut dyn Write,
    err: &mut dyn Write,
    cfg: &crate::config::Config,
) -> Result<i32, String> {
    let (state, warning) = update_state(cfg, fetch)?;
    let empty: Vec<Row> = Vec::new();
    let smh = state.get("SMH").unwrap_or(&empty);
    let qqq = state.get("QQQ").unwrap_or(&empty);
    let soxx = state.get("SOXX").unwrap_or(&empty);
    let (status, details) = classify(smh, qqq, soxx);

    if args.mode == "record" {
        // Accepted even when warned (unlike oilcon). Append, not truncate.
        let path = env.home.join(".nullclaw").join("chipcon-history.log");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let line = record_line(status, &details, warning.as_deref(), now);
        use std::fs::OpenOptions;
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| e.to_string())?;
        f.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
        f.write_all(b"\n").map_err(|e| e.to_string())?;
        let skill = if warning.is_some() { "degraded" } else { "ok" };
        emit_skill_status(skill, env, out);
        emit_trace(env, out);
        return Ok(0);
    }

    let (message, skill_status) =
        format_message(status, &details, cfg, warning.as_deref());
    let code = emit(
        &message,
        skill_status,
        args.deliver_to.as_deref(),
        &args.account,
        env,
        out,
        err,
    );
    Ok(code)
}
