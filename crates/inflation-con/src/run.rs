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
use crate::braking::{self, Cursor};
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
/// Parse argv the way `run.py`'s `argparse` does, **including its refusals**.
///
/// The original port silently ignored unknown flags and accepted any `--mode`
/// value. Not cosmetic: a mistyped `--deliver-to` leaves `deliver_to` at `None`,
/// so the report goes to stdout instead of Telegram and the monthly signal never
/// arrives, while the run still reports `[skill-status:ok]` and the scheduler
/// sees success. This skill fires three times a month, so a silent miss costs a
/// third of the year's coverage before anyone would notice.
///
/// Found 2026-07-31 while cutting chipcon over: `tools/install-skill.sh`'s smoke
/// probe requires exit 2 on an unknown flag, and all three Phase ③ ports failed
/// it while weather and doughcon (Phases ① and ②) passed. The defect tracked the
/// plan, not the implementer — the Phase ③ plans never specified argparse's
/// refusals.
///
/// The exit code is the contract. The message text is not byte-comparable with
/// argparse's usage block and is not attempted.
fn parse_args(argv: &[String], home: &Path) -> Result<Args, String> {
    const MODES: [&str; 3] = ["deliver", "record", "braking"];

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

    // Consume the value belonging to `flag`, or refuse the way argparse does.
    fn value_for(args: &[String], i: &mut usize, flag: &str) -> Result<String, String> {
        *i += 1;
        args.get(*i)
            .cloned()
            .ok_or_else(|| format!("inflation-con: error: argument {flag}: expected one argument"))
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
            "--account" => account = value_for(args, &mut i, "--account")?,
            other => {
                return Err(format!(
                    "inflation-con: error: unrecognized arguments: {other}"
                ));
            }
        }
        i += 1;
    }

    if !MODES.contains(&mode.as_str()) {
        return Err(format!(
            "inflation-con: error: argument --mode: invalid choice: '{mode}' (choose from 'deliver', 'record', 'braking')"
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
    // config_path: CLAW_CONFIG override first (sibling-skill convention), else
    // the Env.home seam — in the binary Env.home IS HOME, so this resolves to
    // claw-core's default path in production while keeping contract tests
    // hermetic (a temp home has no telegram config, so a --deliver-to failure
    // path is exercisable without touching the network or a real account).
    let opts = DeliverOptions {
        account: account.to_string(),
        parse_mode: None,
        config_path: std::env::var("CLAW_CONFIG")
            .ok()
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| Some(env.home.join(".nullclaw").join("config.json"))),
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
    // argparse refuses before doing any work, and so must this: a bad argument
    // must not reach the FRED fetch.
    let args = match parse_args(argv, &env.home) {
        Ok(a) => a,
        Err(msg) => {
            let _ = writeln!(err, "{msg}");
            let _ = err.flush();
            return 2;
        }
    };
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
    // Braking mode is a separate daily run (doc 191 mechanical half): it fetches
    // only its own two series and never touches the inflation series, so it
    // dispatches BEFORE fetch_all — a FEDFUNDS gap must not read as a missing
    // core_pce.
    if args.mode == "braking" {
        return braking_body(args, env, fetch, out, err);
    }

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
    // Doc-191 mechanical half rides along on the monthly report. The braking
    // fetch is deliberately SEPARATE from fetch_all: its failure omits the
    // section and leaves the inflation status untouched (Codex finding 1 —
    // separate runs, no status coupling). The omission stays silent on stderr
    // too: stderr is reserved for run failure by the run.py contract, the
    // omission is visible as the absent section, and the daily braking run
    // is the primary detector anyway.
    let mut message = message;
    let fedfunds = fetch(braking::RULE_SERIES);
    let target = fetch(braking::TARGET_SERIES)
        .ok()
        .and_then(|rows| rows.last().cloned());
    let section = braking::render_monthly_section(
        fedfunds.as_deref().ok().and_then(braking::rule_math).as_ref(),
        fedfunds.as_deref().map(|r| r.len()).unwrap_or(0),
        target.as_ref(),
    );
    if !section.is_empty() {
        message.push_str("\n\n");
        message.push_str(&section);
    }
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

// ── braking mode (doc 191 mechanical half) ──────────────────────────────────

fn braking_cursor_path(env: &Env) -> PathBuf {
    env.home
        .join(".nullclaw")
        .join("skills")
        .join("inflation-con")
        .join("braking-cursor.json")
}

/// Read the cursor. A present-but-corrupt file is a hard error: silently
/// re-baselining would treat an unread change as "first run" and swallow it.
fn read_braking_cursor(path: &Path) -> Result<Option<Cursor>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("braking cursor read {}: {e}", path.display()))?;
    if text.trim().is_empty() {
        return Ok(None);
    }
    Cursor::from_json(&text)
        .map(Some)
        .ok_or_else(|| format!("braking cursor parse {}: refusing to re-baseline", path.display()))
}

fn write_braking_cursor(path: &Path, cursor: &Cursor) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("braking cursor dir: {e}"))?;
    }
    std::fs::write(path, cursor.to_json())
        .map_err(|e| format!("braking cursor write {}: {e}", path.display()))
}

fn braking_body(
    args: &Args,
    env: &Env,
    fetch: &dyn Fn(&str) -> Result<Vec<Obs>, CreditError>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<i32, String> {
    // The timeliness series is primary: hard-fail like core_pce — a silent
    // miss here is a missed Fed move, the exact bug class this mode exists to
    // catch. The rule series stays soft (rule reads "n/a", never a guess).
    let target_rows = match fetch(braking::TARGET_SERIES) {
        Ok(rows) if !rows.is_empty() => rows,
        Ok(_) => {
            return Err(format!(
                "FRED: no DFEDTARU (target upper) observations for {}",
                braking::TARGET_SERIES
            ))
        }
        Err(e) => return Err(format!("fetch {}: {e}", braking::TARGET_SERIES)),
    };
    let rule = fetch(braking::RULE_SERIES)
        .ok()
        .and_then(|rows| braking::rule_math(&rows));

    let path = braking_cursor_path(env);
    let cursor = read_braking_cursor(&path)?;
    let latest = target_rows.last().expect("non-empty (checked above)");

    match braking::detect(cursor.as_ref(), &target_rows) {
        braking::Decision::Baseline { cursor } => {
            // First run: history is not news. Record and exit quietly.
            write_braking_cursor(&path, &cursor)?;
            let _ = writeln!(out, "{}", braking::baseline_line(&cursor));
            let _ = out.flush();
            emit_skill_status("ok", env, out);
            emit_trace(env, out);
            Ok(0)
        }
        braking::Decision::Unchanged => {
            let _ = writeln!(out, "{}", braking::no_op_line(Some(latest)));
            let _ = out.flush();
            emit_skill_status("ok", env, out);
            emit_trace(env, out);
            Ok(0)
        }
        braking::Decision::Changed { from, to, intermediate } => {
            let message =
                braking::render_daily_message(&from, &to, &intermediate, rule.as_ref());
            let code = emit(
                &message,
                "ok",
                args.deliver_to.as_deref(),
                &args.account,
                env,
                out,
                err,
            );
            if code == 0 {
                // Advance only after a successful delivery: a failed send must
                // re-report the change on the next run (Codex finding 2).
                write_braking_cursor(
                    &path,
                    &Cursor {
                        date: to.0.clone(),
                        value: to.1,
                    },
                )?;
            }
            Ok(code)
        }
    }
}
