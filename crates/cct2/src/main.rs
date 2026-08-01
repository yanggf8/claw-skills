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

use cct2::cli::{self, Mode};
use cct2::llm;
use cct2::market;
use cct2::merge::merge;
use cct2::render::format_report;
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

    let tickers = load_tickers();
    let _ = writeln!(err, "[cct2] fetching data for {tickers:?}...");
    let data: Vec<market::TickerData> = tickers.iter().map(|t| market::fetch_ticker(t)).collect();

    let date = jiff::Timestamp::now()
        .in_tz("UTC")
        .map(|z| z.strftime("%Y-%m-%d").to_string())
        .unwrap_or_default();
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
    let mut msg = format_report(&rows, args.mode.as_str(), tickers.len(), &date);

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
