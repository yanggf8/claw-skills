//! doughcon — fetch the PizzINT DOUGHCON level and deliver or record.
//!
//! Exit ownership lives here: claw_core::delivery reports an outcome and this
//! binary decides the exit code, because a hard delivery failure must exit
//! BEFORE markers while a semantic degrade must exit 0 WITH them.

use std::io::Write;

use claw_core::delivery::{deliver, DeliverOptions, DeliveryOutcome};
use claw_core::marker::SkillStatus;
use claw_core::outcome::{finish, Finish};
use doughcon::pizzint;
use doughcon::report::{derive_index, format_body, NO_DATA};
use jiff::{tz::TimeZone, Timestamp, Zoned};

struct Args {
    mode: String,
    deliver_to: Option<String>,
    account: String,
    et_hour: Option<i32>,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args { mode: "deliver".into(), deliver_to: None, account: "main".into(), et_hour: None };
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < argv.len() {
        let need = |i: usize| -> Result<String, String> {
            argv.get(i + 1).cloned().ok_or_else(|| format!("{} requires a value", argv[i]))
        };
        match argv[i].as_str() {
            "--mode" => { a.mode = need(i)?; i += 2; }
            "--deliver-to" => { a.deliver_to = Some(need(i)?); i += 2; }
            "--account" => { a.account = need(i)?; i += 2; }
            // Deliberately NOT range-validated: the Python accepts -1 and 99,
            // which become permanent skips. Pinned by characterization test.
            "--et-hour" => { a.et_hour = Some(need(i)?.parse().map_err(|_| "--et-hour must be an integer".to_string())?); i += 2; }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    if a.mode != "deliver" && a.mode != "record" {
        return Err(format!("--mode must be deliver or record, got {}", a.mode));
    }
    Ok(a)
}

fn history_log_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    std::path::PathBuf::from(home).join(".nullclaw/doughcon-history.log")
}

fn cst_now() -> String {
    let tz = TimeZone::fixed(jiff::tz::offset(8));
    Timestamp::now().to_zoned(tz).strftime("%Y-%m-%d %H:%M:%S CST").to_string()
}

/// API timestamp → Taipei + US-Eastern, minute resolution. Falls back to the
/// run time (SECOND resolution) whenever the timestamp is missing or unparseable.
fn format_updated(raw: Option<&str>) -> String {
    let Some(raw) = raw else { return cst_now() };
    let Ok(ts) = raw.parse::<Timestamp>() else { return cst_now() };
    let (Ok(tpe), Ok(ny)) = (TimeZone::get("Asia/Taipei"), TimeZone::get("America/New_York")) else {
        let tz = TimeZone::fixed(jiff::tz::offset(8));
        return ts.to_zoned(tz).strftime("%Y-%m-%d %H:%M CST").to_string();
    };
    let cst = ts.to_zoned(tpe).strftime("%Y-%m-%d %H:%M CST").to_string();
    let et: Zoned = ts.to_zoned(ny);
    // %Z yields the DST-correct abbreviation (EDT/EST), matching Python's
    // et.tzname(). Verified by compiling against jiff 0.1 — do not hand-roll
    // this from an offset lookup.
    format!("{cst}（美東 {}）", et.strftime("%m-%d %H:%M %Z"))
}

fn main() {
    let mut out = std::io::stdout();
    let mut err = std::io::stderr();

    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => { let _ = writeln!(err, "[ERROR: {e}]"); std::process::exit(2); }
    };

    // DST gate. Fail-open: if tz data is unavailable, warn and run anyway.
    if let Some(target) = args.et_hour {
        match TimeZone::get("America/New_York") {
            Err(_) => { let _ = writeln!(err, "[WARN: --et-hour requires zoneinfo; running unconditionally]"); }
            Ok(ny) => {
                let now = Timestamp::now().to_zoned(ny);
                if now.hour() as i32 != target {
                    // Python appends the tz abbreviation: "... != target 20 (EDT)]".
                    // Dropping it is a live stderr change AND a guaranteed
                    // differential diff on every gate_skip run.
                    let _ = writeln!(
                        err,
                        "[skip: US-Eastern hour {:02} != target {:02} ({})]",
                        now.hour(), target, now.strftime("%Z")
                    );
                    std::process::exit(finish(Finish::Marked { status: SkillStatus::Ok, exit: 0 }, &mut out));
                }
            }
        }
    }

    let base = std::env::var("DOUGHCON_BASE_URL").ok();
    let snapshot = match pizzint::fetch(base.as_deref()) {
        Ok(s) => s,
        Err(e) => {
            if args.mode == "deliver" {
                let msg = format!("[WARN: doughcon unavailable - {e}]");
                let opts = DeliverOptions {
                    account: args.account.clone(),
                    fail_on_delivery_error: false,
                    ..Default::default()
                };
                // Return value deliberately ignored: an upstream failure degrades
                // even if the delivery of that warning also failed.
                let _ = deliver(args.deliver_to.as_deref(), &msg, &opts, &mut out, &mut err);
                std::process::exit(finish(Finish::Marked { status: SkillStatus::Degraded, exit: 0 }, &mut out));
            }
            let _ = writeln!(err, "[ERROR: doughcon unavailable - {e}]");
            std::process::exit(finish(Finish::Unmarked { exit: 1 }, &mut out));
        }
    };

    let index = derive_index(&snapshot.raw_index, &snapshot.popularity_is_null);
    let updated = format_updated(snapshot.timestamp.as_deref());

    if args.mode == "deliver" {
        let job_id = std::env::var("NULLCLAW_JOB_ID").ok().filter(|v| !v.is_empty());
        let body = format_body(&snapshot.level, &index, &updated, job_id.as_deref());
        let opts = DeliverOptions { account: args.account.clone(), ..Default::default() };
        let outcome = deliver(args.deliver_to.as_deref(), &body, &opts, &mut out, &mut err);
        if outcome == DeliveryOutcome::FailedFatal {
            std::process::exit(finish(Finish::Unmarked { exit: 1 }, &mut out));
        }
        let status = if index != NO_DATA { SkillStatus::Ok } else { SkillStatus::Degraded };
        std::process::exit(finish(Finish::Marked { status, exit: 0 }, &mut out));
    }

    let line = format!("{}  DOUGHCON {}  index={}\n", cst_now(), snapshot.level, index);
    match std::fs::OpenOptions::new().create(true).append(true).open(history_log_path()) {
        Ok(mut f) => {
            if let Err(e) = f.write_all(line.as_bytes()) {
                let _ = writeln!(err, "[ERROR: could not write history log - {e}]");
                std::process::exit(finish(Finish::Unmarked { exit: 1 }, &mut out));
            }
        }
        Err(e) => {
            let _ = writeln!(err, "[ERROR: could not write history log - {e}]");
            std::process::exit(finish(Finish::Unmarked { exit: 1 }, &mut out));
        }
    }
    std::process::exit(finish(Finish::Marked { status: SkillStatus::Ok, exit: 0 }, &mut out));
}
