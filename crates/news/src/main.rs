//! news — the daily digest, delivered to Telegram.
//!
//! Ports `news/scripts/run.py`. Three cron jobs a day across three recipients,
//! each either the built-in AI/tech/general feeds or a per-account topic list.
//!
//! Only argument parsing and wiring live here. The delivery run itself —
//! including how its exit code and markers follow the scheduler contract —
//! is `news::run::run_deliver`, so it can be driven offline.

use std::io::Write;

use claw_core::delivery::{deliver, DeliverOptions, DeliveryOutcome};
use claw_core::env::load_env;
use claw_core::marker::SkillStatus;
use claw_core::outcome::{finish, Finish};

use news::cli::{self, Command, DeliverArgs, USAGE};
use news::precheck::new_cache;
use news::run::LiveFeeds;
use news::topics;

fn main() {
    let mut out = std::io::stdout();
    let mut err = std::io::stderr();

    let argv: Vec<String> = std::env::args().skip(1).collect();
    let command = match cli::parse_args(&argv) {
        Ok(c) => c,
        Err(e) => {
            let _ = writeln!(err, "[ERROR: {e}]\n{USAGE}");
            std::process::exit(2);
        }
    };

    load_env(None);

    match command {
        Command::Deliver(args) => std::process::exit(run_deliver(&args, &mut out)),
        other => std::process::exit(run_manage(&other, &mut out, &mut err)),
    }
}

fn run_manage(command: &Command, out: &mut impl Write, err: &mut impl Write) -> i32 {
    let (output, account, deliver_to) = match command {
        Command::Deliver(_) => unreachable!("handled by the caller"),
        Command::ManageList {
            account,
            deliver_to,
        } => (topics::manage_list(account), account, deliver_to),
        Command::ManageAdd {
            account,
            topic,
            deliver_to,
        } => (topics::manage_add(account, topic), account, deliver_to),
        Command::ManageRemove {
            account,
            topic,
            deliver_to,
        } => (topics::manage_remove(account, topic), account, deliver_to),
    };

    let opts = DeliverOptions {
        account: account.clone(),
        ..Default::default()
    };
    if deliver(deliver_to.as_deref(), &output, &opts, out, err) == DeliveryOutcome::FailedFatal {
        return finish(Finish::Unmarked { exit: 1 }, out);
    }
    finish(
        Finish::Marked {
            status: SkillStatus::Ok,
            exit: 0,
        },
        out,
    )
}

fn run_deliver(args: &DeliverArgs, out: &mut impl Write) -> i32 {
    let date_str = jiff::Timestamp::now()
        .in_tz("Asia/Taipei")
        .map(|z| z.strftime("%Y/%m/%d (%a)").to_string())
        .unwrap_or_default();
    news::run::run_deliver(args, &LiveFeeds, &date_str, &new_cache(), out)
}
