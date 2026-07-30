//! Mode dispatch, delivery, and nullclaw marker contract.
//!
//! Line-by-line translation of inflation-con/scripts/run.py `parse_args`,
//! `emit`, and `main`, with writers / Env / fetch / clock injected so the
//! contract goldens can assert without network or process env mutation.
//!
//! Markers are gated on `env.job_id` (the Env seam for NULLCLAW_JOB_ID),
//! matching lib/trace_marker.py: both emit_skill_status and emit_trace are
//! no-ops when the job id is unset. claw-core's marker helpers read the
//! process environment and cannot be used through this seam without races
//! under parallel tests.

use std::io::Write;
use std::path::{Path, PathBuf};

use claw_core::delivery::{deliver, DeliverOptions, DeliveryOutcome};
use market_fetch::fred::CreditError;

use crate::analysis::{classify, Obs, Series};
use crate::config::load_config;
use crate::fetch::fetch_all;
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

/// Parse argv. `argv[0]` is the program name (tests pass `"inflation-con"`).
/// run.py:68-74
fn parse_args(argv: &[String], home: &Path) -> Args {
    let mut mode = "deliver".to_string();
    // run.py:50  DEFAULT_CONFIG = Path.home() / ".nullclaw" / "skills" / "inflation-con" / "config.json"
    let mut config = home
        .join(".nullclaw")
        .join("skills")
        .join("inflation-con")
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

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--mode" => {
                i += 1;
                if i < args.len() {
                    mode = args[i].clone();
                }
            }
            "--config" => {
                i += 1;
                if i < args.len() {
                    config = PathBuf::from(&args[i]);
                    // Python: Path(args.config).expanduser()
                    if let Some(stripped) = args[i].strip_prefix("~/") {
                        config = home.join(stripped);
                    } else if args[i] == "~" {
                        config = home.to_path_buf();
                    }
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

    Args {
        mode,
        config,
        deliver_to,
        account,
    }
}

/// Emit `[skill-status:<status>]` only when job_id is set (manual runs stay clean).
/// lib/trace_marker.py:17-27
fn emit_skill_status(status: &str, env: &Env, out: &mut dyn Write) {
    if env.job_id.is_none() {
        return;
    }
    let _ = writeln!(out, "[skill-status:{status}]");
    let _ = out.flush();
}

/// Emit `[trace:<job_id>]` only when job_id is set.
/// lib/trace_marker.py:30-34
fn emit_trace(env: &Env, out: &mut dyn Write) {
    let Some(ref id) = env.job_id else {
        return;
    };
    let _ = writeln!(out, "[trace:{id}]");
    let _ = out.flush();
}

/// Deliver then markers. Job id is appended bare (not backticks — oilcon differs).
/// run.py:300-311. On hard delivery failure returns 1 without markers
/// (Python deliver_or_fail exits).
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
    // run.py:307-309  if job_id: output += f"\n\n{job_id}"
    if let Some(ref job_id) = env.job_id {
        output.push_str("\n\n");
        output.push_str(job_id);
    }
    // run.py:310  parse_mode=None
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
/// run.py:314-335
pub fn run(
    argv: &[String],
    env: &Env,
    fetch: &dyn Fn(&str) -> Result<Vec<Obs>, CreditError>,
    now: &str,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let args = parse_args(argv, &env.home);
    // load_config runs BEFORE the try — malformed config panics, no markers
    // (wart 1 preserved from run.py:316).
    let cfg = load_config(&args.config);

    match run_body(&args, env, fetch, now, out, err, &cfg) {
        Ok(code) => code,
        Err(e) => {
            // run.py:332  print(f"INFLATION-CON failed: {exc}", file=sys.stderr)
            let _ = writeln!(err, "INFLATION-CON failed: {e}");
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
    fetch: &dyn Fn(&str) -> Result<Vec<Obs>, CreditError>,
    now: &str,
    out: &mut dyn Write,
    err: &mut dyn Write,
    cfg: &crate::config::Config,
) -> Result<i32, String> {
    // run.py:318
    let (state, warning) = fetch_all(&cfg.series, fetch)?;
    let empty: Vec<Obs> = Vec::new();
    let series = Series {
        core_pce: state.get("core_pce").cloned().unwrap_or_else(|| empty.clone()),
        core_cpi: state.get("core_cpi").cloned().unwrap_or_else(|| empty.clone()),
        breakeven_10y: state
            .get("breakeven_10y")
            .cloned()
            .unwrap_or_else(|| empty.clone()),
    };
    // run.py:319
    let (status, details) = classify(&series, &cfg.policy_stance);

    if args.mode == "record" {
        // run.py:320-327 — accepted even when warned (unlike oilcon). Append, not truncate.
        // run.py:321  Path("~/.nullclaw/inflation-con-history.log").expanduser()
        let path = env.home.join(".nullclaw").join("inflation-con-history.log");
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
        // run.py:325  emit_skill_status("degraded" if warning else "ok")
        let skill = if warning.is_some() { "degraded" } else { "ok" };
        emit_skill_status(skill, env, out);
        emit_trace(env, out);
        return Ok(0);
    }

    // run.py:328-330
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
