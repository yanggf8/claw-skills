//! cct — the four CCT trading reports, delivered to Telegram.
//!
//! Ports cct/scripts/run.py, which lived in the cct repo until 2026-08-01 and
//! reached back into this one for its shared lib. The dependency ran the wrong
//! way: a Cloudflare Worker repo importing an agent-skill library. The skill is
//! an agent skill and now sits with the others.
//!
//! Why each mode has its own content predicate. The CCT API answers 200 with
//! `success: true` even when a job never ran or outright failed — the route
//! turns a `status === 'failed'` job into a success envelope carrying only a
//! message. Deciding ok-vs-degraded on "did a payload arrive" would therefore
//! report ok while the pipeline is broken, which is what happened for 50 days.
//! Each empty state has a different shape, so each mode tests for real content.

use std::io::Write;

use cct::api;
use cct::cli::{self, Mode};
use cct::content::{has_eod_data, has_intraday_data, has_pre_market_data, has_weekly_data};
use cct::render::{format_eod, format_intraday, format_pre_market, format_weekly};
use claw_core::delivery::{deliver, DeliverOptions, DeliveryOutcome};
use claw_core::env::load_env;
use claw_core::marker::SkillStatus;
use claw_core::outcome::{finish, Finish};

fn endpoint(m: Mode) -> &'static str {
    match m {
        Mode::PreMarket => "/api/v1/reports/pre-market",
        Mode::Intraday => "/api/v1/reports/intraday",
        Mode::Eod => "/api/v1/reports/end-of-day",
        Mode::Weekly => "/api/v1/reports/weekly",
    }
}

fn label(m: Mode) -> &'static str {
    match m {
        Mode::PreMarket => "盤前報告",
        Mode::Intraday => "盤中報告",
        Mode::Eod => "收盤報告",
        Mode::Weekly => "週報",
    }
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

    let now = jiff::Timestamp::now().in_tz("UTC").expect("UTC");
    let today = now.date();

    let (body, status) = match api::get(endpoint(args.mode)) {
        None => (
            format!("📭 CCT {}尚未產生或暫時無法存取", label(args.mode)),
            SkillStatus::Degraded,
        ),
        Some(data) => {
            let body = match args.mode {
                Mode::PreMarket => format_pre_market(&data, today),
                Mode::Intraday => {
                    format_intraday(&data, &now.strftime("%Y-%m-%d %H:%M UTC").to_string())
                }
                Mode::Eod => format_eod(&data, &now.strftime("%Y-%m-%d").to_string()),
                Mode::Weekly => format_weekly(&data),
            };
            let ok = match args.mode {
                Mode::PreMarket => has_pre_market_data(&data, today),
                Mode::Intraday => has_intraday_data(&data),
                Mode::Eod => has_eod_data(&data),
                Mode::Weekly => has_weekly_data(&data),
            };
            (
                body,
                if ok {
                    SkillStatus::Ok
                } else {
                    SkillStatus::Degraded
                },
            )
        }
    };

    // Degraded still delivers. A stale-but-real report has value, and the
    // "尚未產生" line tells the reader the run happened and found nothing —
    // silence would be indistinguishable from the cron not firing.
    let opts = DeliverOptions {
        account: args.account.clone(),
        ..Default::default()
    };
    let delivery = deliver(args.deliver_to.as_deref(), &body, &opts, &mut out, &mut err);
    if delivery == DeliveryOutcome::FailedFatal {
        std::process::exit(finish(Finish::Unmarked { exit: 1 }, &mut out));
    }

    std::process::exit(finish(Finish::Marked { status, exit: 0 }, &mut out));
}
