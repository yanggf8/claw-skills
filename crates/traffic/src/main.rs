//! traffic — TomTom route travel time between waypoints.
//!
//! Thin binary: parse argv, load env, resolve waypoints, fetch, render,
//! deliver. Ports traffic/scripts/run.py.
//!
//! This skill emits its own scheduler markers, which the Python did not.
//!
//! The Python arrangement was circular: `commute` existed to add
//! `[skill-status:...]` / `[trace:...]`, and `traffic` omitted them only
//! because commute stripped NULLCLAW_JOB_ID out of the child environment
//! before invoking it. Neither could be removed while the other stood. Emitting
//! here breaks the cycle and lets commute be deleted — one skill fewer, one
//! subprocess fewer, and no more deciding ok-vs-degraded by testing whether
//! stdout begins with a car emoji.
//!
//! Markers appear only when NULLCLAW_JOB_ID is set, so a manual run stays
//! clean. The status is read from the outcome, not from the rendered text.
//!
//! Every upstream failure exits 0 with `[WARN: traffic unavailable - ...]` on
//! stdout and reports `degraded`. That line is the delivered message, not a
//! diagnostic — it is what the reader sees when a leg cannot be computed — and
//! exit 0 is deliberate: an unreachable route is a fact about the world, not a
//! broken skill, and a non-zero exit would have the scheduler record
//! exec_error and retry into the same answer.

use std::io::Write;
use std::time::Duration;

use claw_core::agent::call_agent;
use claw_core::delivery::{deliver, DeliverOptions, DeliveryOutcome};
use claw_core::env::load_env;
use claw_core::marker::SkillStatus;
use claw_core::outcome::{finish, Finish};
use traffic::cli::{self, format_advice_line};
use traffic::locations::resolve;
use traffic::render::{body, label, minutes_from_seconds};
use traffic::route;

/// Read `~/.nullclaw/locations.json` into name/coordinate pairs.
///
/// A missing file is not an error — run.py:35 returns `{}` and lets `resolve`
/// report the unknown name, which produces a message naming the file the user
/// has to create.
fn load_locations() -> Vec<(String, String)> {
    let path = match std::env::var("HOME") {
        Ok(home) => format!("{home}/.nullclaw/locations.json"),
        Err(_) => return Vec::new(),
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };
    let parsed: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    parsed
        .as_object()
        .map(|map| {
            map.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

/// Emit the user-facing unavailable line, report `degraded`, and exit 0.
///
/// `degraded` rather than `failed`: the run produced a message and delivered
/// it. Nothing here is repaired by running again — a missing key, an unknown
/// place name and a TomTom outage all return the same answer on a retry — so
/// this is a state to report, not a failure to re-attempt.
fn unavailable(reason: &str, out: &mut impl Write) -> ! {
    let _ = writeln!(out, "[WARN: traffic unavailable - {reason}]");
    std::process::exit(finish(
        Finish::Marked {
            status: SkillStatus::Degraded,
            exit: 0,
        },
        out,
    ));
}

fn main() {
    let mut out = std::io::stdout();
    let mut err = std::io::stderr();

    load_env(None);

    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = match cli::parse_args(&argv) {
        Ok(e) => e,
        Err(e) => {
            // Exit 2 for an argument problem, as argparse does. The installer's
            // smoke probe feeds an undefined flag and requires exactly this.
            let _ = writeln!(err, "[ERROR: {e}]");
            std::process::exit(2);
        }
    };

    // run.py:99-102 checks the key before parsing args. Order does not change
    // the output, and parsing first means a bad flag is still reported as a bad
    // flag on a host with no key configured.
    let api_key = std::env::var("TOMTOM_API_KEY").unwrap_or_default();
    if api_key.is_empty() {
        unavailable("TOMTOM_API_KEY not set", &mut out);
    }

    // Test seam: point the binary at a stub instead of TomTom. Same mechanism
    // as weather's HKO_BASE_URL. Unset in production.
    let base = std::env::var("TOMTOM_BASE_URL").ok().filter(|s| !s.is_empty());

    let table = load_locations();
    let mut waypoints = Vec::new();
    for name in [
        Some(args.origin.as_str()),
        args.via.as_deref(),
        Some(args.dest.as_str()),
    ]
    .into_iter()
    .flatten()
    {
        match resolve(name, &table) {
            Ok(coords) => waypoints.push(coords),
            Err(e) => unavailable(&e.to_string(), &mut out),
        }
    }

    let seconds = match route::fetch(base.as_deref(), &waypoints, &api_key) {
        Ok(s) => s,
        Err(e) => unavailable(&e.to_string(), &mut out),
    };

    let minutes = minutes_from_seconds(seconds);
    let route_label = label(&args.origin, args.via.as_deref(), &args.dest);

    let advice = call_agent(
        &cli::advice_prompt(&route_label, minutes),
        Duration::from_secs(30),
    );
    let advice_line = format_advice_line(&advice).unwrap_or_default();

    let job_id = std::env::var("NULLCLAW_JOB_ID")
        .ok()
        .filter(|v| !v.is_empty());
    let message = body(&route_label, minutes, &advice_line, job_id.as_deref());

    let opts = DeliverOptions {
        account: args.account.clone(),
        ..Default::default()
    };
    let delivery = deliver(
        args.deliver_to.as_deref(),
        &message,
        &opts,
        &mut out,
        &mut err,
    );
    if delivery == DeliveryOutcome::FailedFatal {
        std::process::exit(finish(Finish::Unmarked { exit: 1 }, &mut out));
    }

    std::process::exit(finish(
        Finish::Marked {
            status: SkillStatus::Ok,
            exit: 0,
        },
        &mut out,
    ));
}
