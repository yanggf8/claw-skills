//! Trigger one CCT generation job (manual fallback since 2026-09-14).
//!
//! The schedule itself lives on the worker's own Cloudflare cron triggers
//! (yanggf8/cct wrangler.toml `[triggers]`); before that it was this box's
//! nullclaw cron for two weeks, and GH Actions `schedule:` before that. This
//! binary is the manual fallback and catch-up tool.
//!
//! The trading-day gate keeps that fallback honest: the daily modes refuse
//! to run off-session, because the 2026-09-12 Saturday drain is what wrote
//! the ghost report the pre-market route served for two days. weekly is
//! exempt — Sunday is its scheduled day, and the report looks back over the
//! completed week.
//!
//! Holidays: the worker's own crons still fire on NYSE holidays (parity with
//! the GH Actions era, which also fired into closed markets), so a holiday
//! report exists; this gate only stops the fallback from adding a second,
//! off-schedule generation on top of it.
//!
//! Usage:
//!   cct-trigger pre-market            # -> morning_prediction_alerts
//!   cct-trigger intraday              # -> midday_validation_prediction
//!   cct-trigger eod                   # -> next_day_market_prediction
//!   cct-trigger weekly                # -> weekly_review_analysis
//!   cct-trigger --dry-run pre-market  # print the decision: the POST, or the skip
//!
//! Exit: 0 on success (or an honest skip), 1 on anything else.

use std::io::Write;

use cct::api;
use cct::calendar;

const MODES: &[(&str, &str)] = &[
    ("pre-market", "morning_prediction_alerts"),
    ("intraday", "midday_validation_prediction"),
    ("eod", "next_day_market_prediction"),
    ("weekly", "weekly_review_analysis"),
];

fn trigger_mode(mode: &str) -> &'static str {
    MODES
        .iter()
        .find(|(m, _)| *m == mode)
        .map(|(_, t)| *t)
        .expect("mode is validated before this call")
}

fn main() {
    let mut out = std::io::stdout();
    let mut err = std::io::stderr();

    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut mode: Option<String> = None;
    let mut dry_run = false;
    let mut timeout: u64 = 600;
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "--dry-run" => dry_run = true,
            "--timeout" => match argv.get(i + 1).and_then(|v| v.parse().ok()) {
                Some(t) => {
                    timeout = t;
                    i += 2;
                    continue;
                }
                None => {
                    let _ = writeln!(err, "[ERROR: --timeout requires a number]");
                    std::process::exit(2);
                }
            },
            other if mode.is_none() && MODES.iter().any(|(m, _)| *m == other) => {
                mode = Some(other.to_string());
            }
            other => {
                let _ = writeln!(err, "[ERROR: unknown argument {other}]");
                std::process::exit(2);
            }
        }
        i += 1;
    }
    let mode = match mode {
        Some(m) => m,
        None => {
            let _ = writeln!(
                err,
                "[ERROR: a mode is required (one of: {})]",
                MODES.iter().map(|(m, _)| *m).collect::<Vec<_>>().join(", ")
            );
            std::process::exit(2);
        }
    };

    // Trading-day gate. An unknown holiday year lets weekdays through;
    // `calendar` documents that choice. No fixed-offset fallback: an ET
    // business date is never derived from UTC arithmetic, and jiff bundles
    // the tzdb, so there is no guess to fall back to. A box that cannot
    // resolve its zones should alert, not silently skip — a daily skip would
    // read as "market closed" forever.
    if mode != "weekly" {
        let et_today = jiff::Timestamp::now()
            .in_tz("America/New_York")
            .expect("tzdb is bundled")
            .date();
        if !calendar::is_trading_day(et_today) {
            let _ = writeln!(
                out,
                "✅ cct {mode}: ET {et_today} is not a trading day (weekend/NYSE holiday), skipping trigger"
            );
            std::process::exit(0);
        }
    }

    let tm = trigger_mode(&mode);
    let url = format!("{}/api/v1/jobs/trigger", api::base().trim_end_matches('/'));
    let body = format!(r#"{{"triggerMode":"{tm}"}}"#);

    if dry_run {
        let _ = writeln!(
            out,
            "POST {url} X-API-KEY={} User-Agent={} X-Trigger-Source=nullclaw-cron body={body:?}",
            api::api_key(),
            cct::watchdog::USER_AGENT
        );
        std::process::exit(0);
    }

    let resp = claw_core::http::agent(std::time::Duration::from_secs(timeout))
        .post(&url)
        .set("Content-Type", "application/json")
        .set("X-API-Key", &api::api_key())
        .set("X-Trigger-Source", "nullclaw-cron")
        .set("User-Agent", cct::watchdog::USER_AGENT)
        .send_string(&body);
    let text = match resp {
        Ok(r) => match r.into_string() {
            Ok(t) => t,
            Err(e) => {
                let _ = writeln!(out, "⚠️ cct trigger {mode} failed: response body unreadable ({e})");
                std::process::exit(1);
            }
        },
        Err(ureq::Error::Status(code, r)) => {
            let _ = r.into_string();
            let _ = writeln!(out, "⚠️ cct trigger {mode} failed: HTTP {code}");
            std::process::exit(1);
        }
        Err(e) => {
            let _ = writeln!(out, "⚠️ cct trigger {mode} failed: {e}");
            std::process::exit(1);
        }
    };

    let payload: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => {
            let _ = writeln!(out, "⚠️ cct trigger {mode} failed: response is not JSON ({e})");
            std::process::exit(1);
        }
    };
    if payload.get("success").and_then(|v| v.as_bool()) == Some(true) {
        let run = payload
            .pointer("/data/run_id")
            .or_else(|| payload.pointer("/data/execution_id"))
            .and_then(|v| v.as_str())
            .unwrap_or("ok");
        let _ = writeln!(out, "✅ cct {mode} triggered ({tm}) run={run}");
        std::process::exit(0);
    }
    let head: String = text.chars().take(200).collect();
    let _ = writeln!(out, "⚠️ cct trigger {mode} answered success=false: {head}");
    std::process::exit(1);
}
