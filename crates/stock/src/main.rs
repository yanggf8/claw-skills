//! stock — TWSE and Hang Seng index levels, or one TWSE stock.
//!
//! Ports stock/scripts/run.py.
//!
//! Emits scheduler markers when NULLCLAW_JOB_ID is set, which the Python did
//! not. It has no cron job today, but "no markers because nothing schedules it"
//! is the reasoning that made traffic and commute depend on each other — a
//! skill should be able to report to the scheduler whether or not one is
//! currently listening. A manual run stays clean.
//!
//! Status: `ok` when every requested market answered, `degraded` when at least
//! one did and at least one did not, `failed` when none did. On `failed`
//! nothing is delivered — CLAUDE.md's option A. A retry is the only thing that
//! can then produce a message, so a rescued run sends exactly one rather than
//! following an error with a report.

use std::io::Write;

use claw_core::delivery::{deliver, DeliverOptions, DeliveryOutcome};
use claw_core::env::load_env;
use claw_core::marker::SkillStatus;
use claw_core::outcome::{finish, Finish};
use stock::cli;
use stock::render::line;
use stock::sources::{fetch_twse, fetch_yahoo};

fn env_base(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|s| !s.is_empty())
}

fn main() {
    let mut out = std::io::stdout();
    let mut err = std::io::stderr();

    load_env(None);

    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = match cli::parse_args(&argv) {
        Ok(a) => a,
        Err(e) => {
            let _ = writeln!(err, "[ERROR: {e}]");
            std::process::exit(2);
        }
    };

    let twse_base = env_base("TWSE_BASE_URL");
    let yahoo_base = env_base("YAHOO_BASE_URL");

    let mut lines: Vec<String> = Vec::new();
    let mut wanted = 0usize;
    let mut got = 0usize;

    if let Some(sym) = &args.symbol {
        // A symbol overrides --market, as in run.py:148.
        wanted += 1;
        match fetch_twse(twse_base.as_deref(), &format!("tse_{sym}.tw")) {
            Ok(q) => {
                got += 1;
                lines.push(line(&q));
            }
            Err(e) => lines.push(format!("[WARN: stock {sym} unavailable - {e}]")),
        }
    } else {
        if args.market.wants_tw() {
            wanted += 1;
            match fetch_twse(twse_base.as_deref(), "tse_t00.tw") {
                Ok(q) => {
                    got += 1;
                    lines.push(line(&q));
                }
                Err(e) => lines.push(format!("[WARN: TWSE index unavailable - {e}]")),
            }
        }
        if args.market.wants_hk() {
            wanted += 1;
            match fetch_yahoo(yahoo_base.as_deref(), "%5EHSI", "恒生指數") {
                Ok(q) => {
                    got += 1;
                    lines.push(line(&q));
                }
                Err(e) => lines.push(format!("[WARN: HSI unavailable - {e}]")),
            }
        }
    }

    let status = if got == wanted {
        SkillStatus::Ok
    } else if got > 0 {
        SkillStatus::Degraded
    } else {
        SkillStatus::Failed
    };

    let mut body = lines.join("\n");
    let job_id = std::env::var("NULLCLAW_JOB_ID")
        .ok()
        .filter(|v| !v.is_empty());
    if let Some(id) = &job_id {
        body.push_str(&format!("\n\n`{id}`"));
    }

    // Option A: on a total failure, suppress the chat id so deliver() only
    // echoes to stdout. Nothing reaches Telegram, so the scheduler's retry is
    // the only path that can deliver and a rescued run produces one message.
    let chat = match status {
        SkillStatus::Failed => None,
        _ => args.deliver_to.as_deref(),
    };

    let opts = DeliverOptions {
        account: args.account.clone(),
        ..Default::default()
    };
    let delivery = deliver(chat, &body, &opts, &mut out, &mut err);
    if delivery == DeliveryOutcome::FailedFatal {
        std::process::exit(finish(Finish::Unmarked { exit: 1 }, &mut out));
    }

    std::process::exit(finish(Finish::Marked { status, exit: 0 }, &mut out));
}

