//! One delivery run: fetch, summarize, deliver, classify.
//!
//! Lives in the library rather than `main` so the whole run — the marker
//! included — can be driven offline. Every feed is reached through
//! [`FeedSource`]; `main` passes [`LiveFeeds`], tests pass recorded items.
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

use claw_core::delivery::DeliveryOutcome;
use claw_core::marker::SkillStatus;
use claw_core::outcome::{finish, Finish};
use serde_json::json;

use crate::alert::{alert_failure, AlertContext};
use crate::cache;
use crate::cli::{self, DeliverArgs};
use crate::deliver::deliver_news;
use crate::digest::{summarize_llm_custom, summarize_llm_sourced};
use crate::direct::{direct_ai_from_config, ConfigError, DirectReport, FeedReport};
use crate::feed::{fetch_feed, topic_feed_url};
use crate::precheck::{tier1_filter_items, SharedCache};
use crate::sources::{resolve_ai_sources, AiSources};
use crate::text::{dedup, Item};
use crate::trace::{job_id, log_trace};

const FEED_TIMEOUT: Duration = Duration::from_secs(15);
const FEED_MAX_ITEMS: usize = 15;

/// The four Google News searches the AI section reads in `google` mode.
pub const GOOGLE_AI_FEEDS: [&str; 4] = ["ai_us", "ai_labs", "ai_cn", "ai_tw"];

/// Every upstream a run reads.
pub trait FeedSource {
    /// A built-in Google feed by name (`feed::FEEDS`).
    fn builtin(&self, name: &str) -> Vec<Item>;
    /// One custom topic's Google search.
    fn topic(&self, topic: &str) -> Vec<Item>;
    /// The operator's direct AI feeds (spec §1–§4).
    fn direct_ai(&self) -> Result<(Vec<Item>, DirectReport), ConfigError>;
}

/// The network.
pub struct LiveFeeds;

impl FeedSource for LiveFeeds {
    fn builtin(&self, name: &str) -> Vec<Item> {
        match crate::feed::feed_url(name) {
            Some(url) => fetch_feed(url, FEED_MAX_ITEMS, FEED_TIMEOUT),
            None => Vec::new(),
        }
    }

    fn topic(&self, topic: &str) -> Vec<Item> {
        fetch_feed(&topic_feed_url(topic), 10, FEED_TIMEOUT)
    }

    /// `~/.nullclaw/news-ai-feeds.json`, fetched against the real clock.
    fn direct_ai(&self) -> Result<(Vec<Item>, DirectReport), ConfigError> {
        direct_ai_from_config(None, jiff::Timestamp::now())
    }
}

/// Raw deduped items per topic. Tier-1 filtering is applied by the caller,
/// *after* the feed-emptiness check, so a deny list that empties every topic is
/// not misread as a feed outage.
fn fetch_custom_topics(topics: &[String], feeds: &dyn FeedSource) -> Vec<(String, Vec<Item>)> {
    topics
        .iter()
        .map(|topic| (topic.clone(), dedup(&feeds.topic(topic))))
        .collect()
}

/// One feed's line in the `ai_direct_feeds_empty` detail.
fn feed_outcome_line(f: &FeedReport) -> String {
    let outcome = serde_json::to_value(f.outcome)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default();
    let detail = f
        .detail
        .as_deref()
        .map(|d| format!("({d})"))
        .unwrap_or_default();
    let c = &f.counts;
    let mut parts = vec![
        format!("{}={outcome}{detail}", f.name),
        format!("fetched={}", c.fetched),
    ];
    for (name, n) in [
        ("malformed", c.malformed),
        ("stale", c.stale),
        ("undated", c.undated),
        ("future", c.future),
        ("invalid_link", c.invalid_link),
    ] {
        if n > 0 {
            parts.push(format!("{name}={n}"));
        }
    }
    parts.push(format!("kept={}", c.kept));
    parts.join(" ")
}

/// The direct AI candidates, or none.
///
/// An AI-wide outage [R7] — no usable config, or every feed failed, timed
/// out or filtered to zero — raises `ai_direct_feeds_empty` and leaves the
/// section empty, so it renders `今日無相關新聞` while tech and 重大新聞 ship.
/// Never a fallback to Google: that would hide a dead feed list behind a
/// working digest.
pub fn direct_ai_items(ctx: &AlertContext, feeds: &dyn FeedSource) -> Vec<Item> {
    match feeds.direct_ai() {
        Err(e) => {
            alert_failure(
                ctx,
                "ai_direct_feeds_empty",
                &format!(
                    "direct AI feed config {} ({e}); AI section empty, google not used",
                    e.code()
                ),
            );
            Vec::new()
        }
        Ok((_, report)) if report.total_kept == 0 => {
            let per_feed: Vec<String> = report.feeds.iter().map(feed_outcome_line).collect();
            alert_failure(
                ctx,
                "ai_direct_feeds_empty",
                &format!(
                    "direct AI feeds kept 0 items; AI section empty, google not used. {}",
                    per_feed.join("; ")
                ),
            );
            Vec::new()
        }
        Ok((items, _)) => items,
    }
}

/// The default digest's raw feeds, the AI section's from `sources`.
pub fn fetch_default_feeds(
    sources: AiSources,
    ctx: &AlertContext,
    feeds: &dyn FeedSource,
) -> Vec<(String, Vec<Item>)> {
    let ai = match sources {
        AiSources::Google => {
            let mut ai: Vec<Item> = Vec::new();
            for name in GOOGLE_AI_FEEDS {
                ai.extend(feeds.builtin(name));
            }
            ai
        }
        AiSources::Direct => direct_ai_items(ctx, feeds),
    };
    vec![
        ("ai".to_string(), dedup(&ai)),
        ("tech".to_string(), dedup(&feeds.builtin("tech"))),
        ("general".to_string(), dedup(&feeds.builtin("general"))),
    ]
}

/// Fetch, summarize, deliver and classify one run. Returns the exit code.
///
/// `NEWS_AI_SOURCES` is read here, per run, and only for the default digest:
/// a custom-topic digest has no AI section.
pub fn run_deliver(
    args: &DeliverArgs,
    feeds: &dyn FeedSource,
    date_str: &str,
    cache_handle: &SharedCache,
    out: &mut impl Write,
) -> i32 {
    // Opportunistic, before any heavy work.
    cache::sweep();

    let ctx = AlertContext::new(
        args.deliver_to.clone(),
        args.account.clone(),
        std::env::var("NULLCLAW_JOB_ID").ok(),
    );

    let run_start = Instant::now();
    let mut outcome = "ok";
    let feeds_ms: Option<u64>;
    let mut summarize_ms: Option<u64> = None;
    let mut deliver_ms: Option<u64> = None;

    let code = {
        let topic_list = cli::resolve_topics(args);
        let sources = match topic_list {
            Some(_) => AiSources::Google,
            None => resolve_ai_sources(&ctx),
        };

        let t = Instant::now();
        let all_items = match topic_list.as_deref() {
            Some(topics) => fetch_custom_topics(topics, feeds),
            None => fetch_default_feeds(sources, &ctx, feeds),
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
                    date_str,
                    cache_handle,
                )),
                // `None` here means the AI section was exhausted. The alert
                // already went out from inside the substage path.
                None => {
                    summarize_llm_sourced(&all_items, &ctx, date_str, cache_handle, sources).ok()
                }
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
                        // `ok` even when an alert went out above — an invalid
                        // NEWS_AI_SOURCES, a direct AI-wide outage, a section
                        // fallback. Each of those still delivered a digest, and
                        // `degraded` is `verified != 1`, which `retry_once`
                        // answers by delivering it a second time.
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
    let (elapsed, remaining) = crate::agent::skill_wallclock();
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
