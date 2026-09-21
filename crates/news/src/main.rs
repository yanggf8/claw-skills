//! news — the daily digest, delivered to Telegram.
//!
//! Ports `news/scripts/run.py`. Three cron jobs a day across three recipients,
//! each either the built-in AI/tech/general feeds or a per-account topic list.
//!
//! The scheduler contract shapes the exits. A non-zero exit wins over every
//! marker in nullclaw's classification, so the two hard-failure paths — no
//! feeds at all, and an AI section that could not be produced even after
//! subdividing — exit 1 and deliver nothing. That is deliberate: the cron
//! alert is the right channel for "the skill produced nothing", and it leaves
//! a retry as the only thing that can deliver, so a rescued run sends exactly
//! one message.

use std::io::Write;
use std::time::{Duration, Instant};

use claw_core::delivery::{deliver, DeliverOptions, DeliveryOutcome};
use claw_core::env::load_env;
use claw_core::marker::SkillStatus;
use claw_core::outcome::{finish, Finish};
use serde_json::json;

use news::alert::{alert_failure, AlertContext};
use news::cache;
use news::cli::{self, Command, DeliverArgs, USAGE};
use news::digest::{summarize_llm, summarize_llm_custom};
use news::feed::{fetch_feed, topic_feed_url};
use news::deliver::deliver_news;
use news::precheck::{new_cache, tier1_filter_items};
use news::text::{dedup, Item};
use news::topics;
use news::trace::{job_id, log_trace};

const FEED_TIMEOUT: Duration = Duration::from_secs(15);
const FEED_MAX_ITEMS: usize = 15;

fn fetch(name: &str) -> Vec<Item> {
    match news::feed::feed_url(name) {
        Some(url) => fetch_feed(url, FEED_MAX_ITEMS, FEED_TIMEOUT),
        None => Vec::new(),
    }
}

/// Raw deduped items per topic. Tier-1 filtering is applied by the caller,
/// *after* the feed-emptiness check, so a deny list that empties every topic is
/// not misread as a feed outage.
fn fetch_custom_topics(topics: &[String]) -> Vec<(String, Vec<Item>)> {
    topics
        .iter()
        .map(|topic| {
            let items = fetch_feed(&topic_feed_url(topic), 10, FEED_TIMEOUT);
            (topic.clone(), dedup(&items))
        })
        .collect()
}

fn fetch_default_feeds() -> Vec<(String, Vec<Item>)> {
    let mut ai: Vec<Item> = Vec::new();
    for name in ["ai_us", "ai_labs", "ai_cn", "ai_tw"] {
        ai.extend(fetch(name));
    }
    vec![
        ("ai".to_string(), dedup(&ai)),
        ("tech".to_string(), dedup(&fetch("tech"))),
        ("general".to_string(), dedup(&fetch("general"))),
    ]
}

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
    // Opportunistic, before any heavy work.
    cache::sweep();

    let ctx = AlertContext::new(
        args.deliver_to.clone(),
        args.account.clone(),
        std::env::var("NULLCLAW_JOB_ID").ok(),
    );
    let cache_handle = new_cache();

    let run_start = Instant::now();
    let mut outcome = "ok";
    let feeds_ms: Option<u64>;
    let mut summarize_ms: Option<u64> = None;
    let mut deliver_ms: Option<u64> = None;

    let date_str = jiff::Timestamp::now()
        .in_tz("Asia/Taipei")
        .map(|z| z.strftime("%Y/%m/%d (%a)").to_string())
        .unwrap_or_default();

    let code = {
        let topic_list = cli::resolve_topics(args);

        let t = Instant::now();
        let all_items = match topic_list.as_deref() {
            Some(topics) => fetch_custom_topics(topics),
            None => fetch_default_feeds(),
        };
        feeds_ms = Some(t.elapsed().as_millis() as u64);

        // Feed outage is decided on the RAW deduped feeds, before Tier 1. A
        // deny list that empties every section is a filter outcome, not an
        // outage, and must not trigger the alert or the non-zero exit.
        let has_items = all_items.iter().any(|(_, v)| !v.is_empty());
        let all_items: Vec<(String, Vec<Item>)> = all_items
            .into_iter()
            .map(|(k, v)| (k, tier1_filter_items(v)))
            .collect();

        if !has_items {
            outcome = "feeds_empty";
            alert_failure(
                &ctx,
                "all_feeds_empty",
                "every RSS feed returned 0 items — likely network failure or feed outage",
            );
            // Unmarked: nullclaw's non-zero-exit branch wins over any marker,
            // so one emitted here is noise the classifier never reads.
            finish(Finish::Unmarked { exit: 1 }, out)
        } else {
            let t = Instant::now();
            let summary = match topic_list.as_deref() {
                Some(topics) => Some(summarize_llm_custom(
                    &all_items,
                    topics,
                    &ctx,
                    &date_str,
                    &cache_handle,
                )),
                // `None` here means the AI section was exhausted. The alert
                // already went out from inside the substage path.
                None => summarize_llm(&all_items, &ctx, &date_str, &cache_handle).ok(),
            };
            summarize_ms = Some(t.elapsed().as_millis() as u64);

            match summary {
                None => {
                    outcome = "ai_exhausted";
                    finish(Finish::Unmarked { exit: 1 }, out)
                }
                Some(summary) => {
                    let job = job_id();
                    let summary = if job == "interactive" {
                        summary
                    } else {
                        format!("{summary}\n\n`{job}`")
                    };

                    let t = Instant::now();
                    let mut derr = std::io::stderr();
                    let delivery = deliver_news(
                        args.deliver_to.as_deref(),
                        &summary,
                        &args.account,
                        None,
                        out,
                        &mut derr,
                    );
                    deliver_ms = Some(t.elapsed().as_millis() as u64);

                    if delivery == DeliveryOutcome::FailedFatal {
                        // The body reached stdout for the cron capture but not
                        // Telegram. The on-disk failure log records this even
                        // when Telegram is itself the dead channel.
                        outcome = "delivery_failed";
                        alert_failure(
                            &ctx,
                            "telegram_delivery_failed",
                            "delivery returned a fatal failure — telegram send did not succeed",
                        );
                        finish(Finish::Unmarked { exit: 1 }, out)
                    } else {
                        finish(
                            Finish::Marked {
                                status: SkillStatus::Ok,
                                exit: 0,
                            },
                            out,
                        )
                    }
                }
            }
        }
    };

    // On every path, so a day of traces can answer how close the run got to its
    // timeout and where the time went. A hard SIGKILL leaves no entry at all —
    // treat a missing one near the timeout as a probable kill.
    let (elapsed, remaining) = news::agent::skill_wallclock();
    log_trace(
        "news_run_timing",
        json!({
            "outcome": outcome,
            "total_ms": run_start.elapsed().as_millis() as u64,
            "feeds_ms": feeds_ms,
            "summarize_ms": summarize_ms,
            "deliver_ms": deliver_ms,
            "skill_timeout": std::env::var("NULLCLAW_SKILL_TIMEOUT").ok(),
            "elapsed_since_skill_start": elapsed,
            "remaining_to_kill": remaining,
        }),
    );
    code
}
