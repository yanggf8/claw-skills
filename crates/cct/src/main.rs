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
use cct::content::content_gap;
use cct::freshness::comparison_today;
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

    // One instant, read in two zones. Calling `now()` twice would leave a
    // midnight race where the two dates come from different days — the same
    // shape of defect this whole change is about, one layer down.
    //
    // ET is the market's own time, so the trading day IS the ET date, and it is
    // also what the reports are stamped with. jiff is built with
    // `tzdb-bundle-always`, so the zone needs nothing from the host and this
    // works inside the nanoclaw container too.
    let instant = jiff::Timestamp::now();
    let now = instant.in_tz("UTC").expect("UTC");
    let now_et = instant.in_tz("America/New_York").expect("tzdb is bundled");
    let utc_today = now.date();
    let et_today = now_et.date();

    let (body, status) = match api::get(endpoint(args.mode)) {
        None => (
            format!("📭 CCT {}尚未產生或暫時無法存取", label(args.mode)),
            SkillStatus::Degraded,
        ),
        Some(report) => {
            // The clock follows the field that was read — see
            // `freshness::comparison_today` for why the two travel together.
            let today = comparison_today(report.business_date.as_deref(), et_today, utc_today);
            let body = match args.mode {
                Mode::PreMarket => format_pre_market(&report.data, today),
                Mode::Intraday => {
                    format_intraday(&report.data, &now_et.strftime("%Y-%m-%d %H:%M ET").to_string())
                }
                Mode::Eod => format_eod(
                    report.business_date.as_deref(),
                    &report.data,
                    &now_et.strftime("%Y-%m-%d").to_string(),
                ),
                Mode::Weekly => format_weekly(&report.data),
            };
            // A degraded verdict reached this way used to be silent, and
            // nullclaw prints the literal "no stderr" in the alert when a skill
            // writes none (gateway.zig, the degraded branch). The envelope
            // warnings cover only the other fork — the one where no payload
            // arrives — so an intact payload with nothing in it alerted with no
            // reason at all on 2026-08-07.
            // `has_content: false` is the worker's own answer about its own
            // storage, so it wins over any predicate a reader can apply to the
            // shape of a payload the route synthesised. The reverse is refused
            // deliberately: `true` does not silence the predicates, because they
            // are what caught a dead pipeline serving plausible reports for 50
            // days, and a field that could switch them off would hand that
            // failure a way back in.
            let gap = match report.has_content {
                Some(false) => Some(format!(
                    "the worker has no {} content for {}",
                    args.mode.slug(),
                    report.business_date.as_deref().unwrap_or("the day requested"),
                )),
                _ => content_gap(args.mode, &report.data, today),
            };
            if let Some(reason) = &gap {
                let _ = writeln!(
                    err,
                    "[WARN: CCT {} carries no analysis] {reason}",
                    args.mode.slug()
                );
            }
            (
                body,
                if gap.is_none() {
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
