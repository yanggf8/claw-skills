//! cct2 — dual-LLM market sentiment, pre-market and end-of-day.
//!
//! Ports cct2/scripts/run.py, and fixes two faults that compounded:
//!
//! 1. The primary model was called through
//!    `nullclaw agent --provider anthropic-custom:minimax`, which nullclaw
//!    rejects — `minimax` is not an absolute URL — so it never answered.
//! 2. A ticker only one model answered was marked `consensus: not both_present`,
//!    i.e. true, so the report filed every backup-only reading under
//!    🎯 共識訊號 and footed it "雙模型對照".
//!
//! Together they meant every report claimed a two-model consensus while one
//! model had never been reached. Both models are now called over plain HTTPS —
//! both endpoints speak the Anthropic messages API — and agreement is a
//! three-way enum that cannot say "consensus" about one voice.

use std::io::Write;

use cct2::clock::{self, Gate};
use cct2::cli::{self, Mode};
use cct2::journal::{self, Journal};
use cct2::llm;
use cct2::market;
use cct2::merge::merge;
use cct2::render::{format_report, ReportContext};
use cct2::review;
use claw_core::delivery::{deliver, DeliverOptions, DeliveryOutcome};
use claw_core::env::load_env;
use claw_core::marker::SkillStatus;
use claw_core::outcome::{finish, Finish};

const DEFAULT_TICKERS: [&str; 5] = ["AAPL", "MSFT", "GOOGL", "TSLA", "NVDA"];

const PROMPT: &str = r#"You are a financial analyst. Analyze the following market data and give your sentiment for each stock.

Mode: {mode}
Date: {date}

{ticker_data}

For each ticker, reply with ONLY a JSON object (no markdown, no explanation) like:
{
  "AAPL": {"sentiment": "bullish", "confidence": 0.82, "reason": "one sentence"},
  "MSFT": {"sentiment": "bearish", "confidence": 0.71, "reason": "one sentence"},
  ...
}

Rules:
- sentiment must be exactly: bullish, bearish, or neutral
- confidence is 0.0 to 1.0
- reason is one concise sentence under 80 characters
- Reply with the JSON object only, nothing else
"#;

/// Tickers from nullclaw memory, else the default five.
fn load_tickers() -> Vec<String> {
    if let Ok(o) = std::process::Command::new("nullclaw")
        .args(["memory", "get", "cct2:tickers"])
        .output()
    {
        if o.status.success() {
            let raw = String::from_utf8_lossy(&o.stdout).trim().to_string();
            // "cct2:tickers: AAPL MSFT …" — drop the key prefix if present.
            let body = raw.split_once(':').map(|(_, r)| r).unwrap_or(&raw);
            let list: Vec<String> = body
                .split_whitespace()
                .map(|t| t.trim().to_uppercase())
                .filter(|t| !t.is_empty())
                .collect();
            if !list.is_empty() {
                return list;
            }
        }
    }
    DEFAULT_TICKERS.iter().map(|s| s.to_string()).collect()
}

fn skill_config() -> serde_json::Value {
    let path = std::env::var("CCT2_CONFIG").unwrap_or_else(|_| {
        format!(
            "{}/.nullclaw/skills/cct2/config.json",
            std::env::var("HOME").unwrap_or_default()
        )
    });
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(serde_json::Value::Null)
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

    let cfg = skill_config();
    let model_of = |key: &str, default: &str| {
        cfg.get(key)
            .and_then(|v| v.as_str())
            .unwrap_or(default)
            .to_string()
    };
    let primary_model = model_of("primary_model", llm::DEFAULT_PRIMARY_MODEL);
    let backup_model = model_of("backup_model", llm::DEFAULT_BACKUP_MODEL);

    // Market time, read once. Everything dated below is the ET trading day.
    let now = clock::market_now();
    if now.is_none() {
        let _ = writeln!(err, "[cct2] WARN no zoneinfo; dates fall back to empty");
    }

    // DST gate, before any network work so a skipped run costs nothing. Fails
    // open: without tz data there is no hour to compare, and refusing to run
    // would turn a missing database into a silently missing report.
    if let (Some(target), Some(z)) = (args.et_hour, now.as_ref()) {
        let abbrev = z.strftime("%Z").to_string();
        if let Gate::Skip {
            current_hour,
            abbrev,
        } = clock::gate(z.hour() as i32, &abbrev, Some(target))
        {
            let _ = writeln!(
                err,
                "[skip: US-Eastern hour {current_hour:02} != target {target:02} ({abbrev})]"
            );
            std::process::exit(finish(
                Finish::Marked {
                    status: SkillStatus::Ok,
                    exit: 0,
                },
                &mut out,
            ));
        }
    } else if args.et_hour.is_some() {
        let _ = writeln!(err, "[WARN: --et-hour requires zoneinfo; running unconditionally]");
    }

    let date = now.as_ref().map(clock::business_date).unwrap_or_default();
    let market_time = now.as_ref().map(clock::market_stamp).unwrap_or_default();
    let home = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default());

    let tickers = load_tickers();
    let _ = writeln!(err, "[cct2] fetching data for {tickers:?}...");
    let data: Vec<market::TickerData> = tickers.iter().map(|t| market::fetch_ticker(t)).collect();
    let price_of = |t: &str| -> Option<f64> {
        data.iter()
            .find(|d| d.ticker == t)
            .and_then(|d| d.quote.as_ref())
            .map(|q| q.price)
    };

    // The close is reviewed against the morning's record for the same trading
    // day. A missing journal is normal — the pre-market run may have been
    // skipped or failed — and renders no review section rather than an error.
    let (reviewed, review_made_at) = match args.mode {
        Mode::Eod => match journal::load(&home, &date) {
            Some(j) => {
                let _ = writeln!(
                    err,
                    "[cct2] reviewing {} pre-market prediction(s) from {}",
                    j.predictions.len(),
                    j.business_date
                );
                (review::review(&j.predictions, &price_of), j.made_at)
            }
            None => {
                let _ = writeln!(err, "[cct2] no pre-market journal for {date}; review omitted");
                (Vec::new(), String::new())
            }
        },
        Mode::PreMarket => (Vec::new(), String::new()),
    };

    let mode_label = match args.mode {
        Mode::PreMarket => "pre-market (before US open, give outlook for today's session)",
        Mode::Eod => "end-of-day (market just closed, summarize today and give tomorrow's outlook)",
    };
    let prompt = PROMPT
        .replace("{mode}", mode_label)
        .replace("{date}", &date)
        .replace("{ticker_data}", &market::summarise(&data));

    let (primary_ep, backup_ep) = llm::endpoints(&primary_model, &backup_model);
    if primary_ep.is_none() {
        let _ = writeln!(err, "[cct2] WARN no primary key configured");
    }
    if backup_ep.is_none() {
        let _ = writeln!(err, "[cct2] WARN no backup key configured");
    }

    let _ = writeln!(err, "[cct2] querying dual LLM...");
    let (primary, backup) = llm::run_dual(&prompt, primary_ep.as_ref(), backup_ep.as_ref());

    let rows = merge(&tickers, primary.as_ref(), backup.as_ref());

    // Record before rendering, and only when there is something to record: the
    // journal is what tonight's review reads, and a run that produced no rows
    // must not overwrite a good morning record with an empty one.
    if args.mode == Mode::PreMarket && !rows.is_empty() && !date.is_empty() {
        let entry = Journal {
            business_date: date.clone(),
            made_at: market_time.clone(),
            predictions: journal::predictions_from(&rows, &price_of),
        };
        match journal::save(&home, &entry) {
            Ok(p) => {
                let _ = writeln!(err, "[cct2] journal written: {}", p.display());
            }
            // A failed write costs tonight's review, not this report.
            Err(e) => {
                let _ = writeln!(err, "[cct2] WARN journal write failed: {e}");
            }
        }
    }

    let mut msg = format_report(
        &rows,
        &ReportContext {
            mode: args.mode.as_str(),
            ticker_count: tickers.len(),
            date: &date,
            market_time: &market_time,
            review: &reviewed,
            review_made_at: &review_made_at,
        },
    );

    let job_id = std::env::var("NULLCLAW_JOB_ID")
        .ok()
        .filter(|v| !v.is_empty());
    if let Some(id) = &job_id {
        msg.push_str(&format!("\n\n`{id}`"));
    }

    // Option A: no rows means no message. The scheduler retries any run ending
    // verified != 1, re-execing with an identical environment, so a skill that
    // delivers here cannot tell the retry from the original and the rescued run
    // adds a second message. Suppressing the chat id leaves the body on stdout
    // for cron_runs.output and spares Telegram.
    let status = if rows.is_empty() {
        SkillStatus::Failed
    } else {
        SkillStatus::Ok
    };
    let chat = if rows.is_empty() {
        None
    } else {
        args.deliver_to.as_deref()
    };

    let opts = DeliverOptions {
        account: args.account.clone(),
        ..Default::default()
    };
    let delivery = deliver(chat, &msg, &opts, &mut out, &mut err);
    if delivery == DeliveryOutcome::FailedFatal {
        std::process::exit(finish(Finish::Unmarked { exit: 1 }, &mut out));
    }

    std::process::exit(finish(Finish::Marked { status, exit: 0 }, &mut out));
}
