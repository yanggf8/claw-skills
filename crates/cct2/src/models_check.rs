//! Durable acceptance for the models journal: did the last business day get
//! both modes, with the primary model answering?
//!
//! `models.jsonl` is the per-run record of who answered (primary and backup,
//! per mode, with a ticker count). This module is the verdict over one day's
//! rows — the `cct2-check` binary adds the file plumbing around it. A primary
//! that goes quiet while the backup answers still ships a report (the reader
//! marks it 單一模型回應), so this check exists to surface exactly that class
//! as a cron alert instead of a grep someone has to remember.
//!
//! `expected_day` is the last ET weekday the journal should reach. The check
//! has no holiday table — cct2's own schedule is weekday-based and fires on
//! holidays too, so a holiday writes a row and never reads as stale. A
//! journal that stops short of the expected day says so explicitly: a
//! repeated verdict about a day nothing new landed on must name itself as
//! staleness, not masquerade as a fresh model failure (2026-09-11: the box
//! outage stopped the journal at 09-10, and the old checker re-reported
//! 09-10's primary miss every following night as if it were new).

use serde_json::Value;

#[derive(Debug, PartialEq)]
pub struct Verdict {
    pub ok: bool,
    pub reasons: Vec<String>,
}

/// `entries` are the parsed JSONL rows, in file order.
pub fn evaluate(entries: &[Value], expected_day: Option<&str>) -> Verdict {
    let mut ok = true;
    let mut reasons = Vec::new();

    let latest_date = entries
        .last()
        .and_then(|e| e.get("business_date"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let latest: Vec<&Value> = entries
        .iter()
        .filter(|e| e.get("business_date").and_then(|v| v.as_str()) == Some(latest_date.as_str()))
        .collect();

    let mut modes: Vec<&str> = latest
        .iter()
        .filter_map(|e| e.get("mode").and_then(|v| v.as_str()))
        .collect();
    modes.sort_unstable();
    modes.dedup();

    for e in &latest {
        let mode = e.get("mode").and_then(|v| v.as_str()).unwrap_or("?");
        let primary = e.get("primary");
        if primary.and_then(|p| p.get("answered")).and_then(|v| v.as_bool()) != Some(true) {
            ok = false;
            reasons.push(format!("{mode} primary not answered"));
        }
        let tickers = primary
            .and_then(|p| p.get("tickers"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let requested = e.get("requested").and_then(|v| v.as_i64()).unwrap_or(0);
        if tickers < requested {
            ok = false;
            reasons.push(format!("{mode} primary tickers {tickers} < requested {requested}"));
        }
    }
    for mode in ["pre-market", "eod"] {
        if !modes.contains(&mode) {
            ok = false;
            reasons.push(format!("{mode} missing for {latest_date}"));
        }
    }

    if let Some(expected) = expected_day {
        if latest_date.as_str() < expected {
            ok = false;
            reasons.push(format!(
                "journal is stale: last entry {latest_date}, expected {expected}"
            ));
        }
    }

    Verdict { ok, reasons }
}

/// The last ET weekday at or before `today` — the day the journal should
/// reach. Weekends scan back to Friday; a holiday stays a weekday here by
/// design (see the module doc).
pub fn last_expected_weekday(today: jiff::civil::Date) -> jiff::civil::Date {
    use jiff::{civil::Weekday, ToSpan};
    let mut d = today;
    while matches!(d.weekday(), Weekday::Saturday | Weekday::Sunday) {
        d -= 1.day();
    }
    d
}
