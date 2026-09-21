//! Whether a payload carries real analysis, per mode — and why not.

use crate::cli::Mode;
use crate::freshness::pre_market_freshness;
use jiff::civil::Date;

fn nonempty_array(v: Option<&serde_json::Value>) -> bool {
    v.and_then(|x| x.as_array()).is_some_and(|a| !a.is_empty())
}

fn truthy(v: Option<&serde_json::Value>) -> bool {
    match v {
        None | Some(serde_json::Value::Null) => false,
        Some(serde_json::Value::Bool(b)) => *b,
        Some(serde_json::Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
        Some(serde_json::Value::String(s)) => !s.is_empty(),
        Some(serde_json::Value::Array(a)) => !a.is_empty(),
        Some(serde_json::Value::Object(o)) => !o.is_empty(),
    }
}

/// Real analysis content, for today.
///
/// Content alone proves nothing: the D1 fallback serves a complete set of
/// signals from whatever day last succeeded, so a stale payload is
/// indistinguishable from a fresh one except by date. Stale counts as degraded
/// rather than failed for the same reason an empty payload does — a retry
/// returns the same snapshot.
pub fn has_pre_market_data(data: &serde_json::Value, today: Date) -> bool {
    if pre_market_freshness(data, today).is_stale {
        return false;
    }
    nonempty_array(data.get("high_confidence_signals"))
        || truthy(data.get("symbols_analyzed"))
        || truthy(data.get("overall_sentiment"))
}

/// Real end-of-day content, in either shape the route can serve.
///
/// The live payload is a prediction scorecard (flat camelCase). `daily_summary`
/// only ever appears in the placeholder the route synthesises when it finds no
/// snapshot, so testing that alone reported degraded on every genuine report —
/// which is what happened from 2026-07-21 onward. Check both.
pub fn has_eod_data(data: &serde_json::Value) -> bool {
    if truthy(data.get("signalBreakdown"))
        || truthy(data.get("totalSignals"))
        || truthy(data.get("symbols_analyzed"))
    {
        return true;
    }
    let summary = data.get("daily_summary");
    truthy(summary.and_then(|s| s.get("symbols_analyzed")))
        || truthy(data.get("high_confidence_signals"))
}

pub fn has_intraday_data(data: &serde_json::Value) -> bool {
    truthy(data.get("total_symbols")) || truthy(data.get("symbols"))
}

pub fn has_weekly_data(data: &serde_json::Value) -> bool {
    let report = data.get("report").unwrap_or(data);
    if !report.is_object() {
        return false;
    }
    truthy(report.get("weekly_overview")) || truthy(report.get("daily_breakdown"))
}

/// How much upstream text a reason may quote, in bytes.
///
/// nullclaw cuts the alert preview at 200 **bytes**, not characters
/// (gateway.zig, the degraded branch). Two things follow. An unbounded quote
/// pushes the rest of the reason out of the preview, so a route that leads with
/// 500 characters of boilerplate would hide the sentence worth reading. And a
/// cut landing mid-codepoint hands Telegram invalid UTF-8, which loses the whole
/// alert — the quoted string is upstream-controlled and need not be ASCII.
/// 120 bytes keeps the longest warning line ("[WARN: CCT pre-market carries no
/// analysis] " is 43) under the preview, so the cut never fires on it at all.
const QUOTE_MAX_BYTES: usize = 120;

/// Upstream text, made safe to put in a log line and an alert.
///
/// Control characters become spaces — a newline would split the warning across
/// lines in the operator's alert, and a NUL is worse. Truncation is on a
/// character boundary.
fn tidy(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.len() <= QUOTE_MAX_BYTES {
        return trimmed.to_string();
    }
    let mut kept = String::with_capacity(QUOTE_MAX_BYTES + 3);
    for c in trimmed.chars() {
        if kept.len() + c.len_utf8() > QUOTE_MAX_BYTES {
            break;
        }
        kept.push(c);
    }
    kept.push('…');
    kept
}

/// A non-empty string field, for quoting the route back at the reader.
fn quoted(v: Option<&serde_json::Value>) -> Option<String> {
    v.and_then(|x| x.as_str())
        .map(tidy)
        .filter(|s| !s.is_empty())
}

/// Why this payload counts as empty, or `None` when it carries analysis.
///
/// Paired with the predicates above and returning `Some` exactly when the
/// mode's `has_*_data` returns false, so the reason and the verdict cannot
/// drift apart. `content_reason.rs` pins that agreement.
///
/// The reason exists because a degraded alert without one is unreadable: on
/// 2026-08-07 the eod job alerted `failure=contract_degraded … no stderr` while
/// the route was up, the envelope was well-formed and the payload was the
/// placeholder the worker synthesises when no snapshot exists. A dead upstream
/// job, a stale-but-real report and an unreachable host all reached the
/// operator as the same sentence. Prefer the route's own words — they name the
/// next step ("Run POST /api/v1/jobs/intraday") more precisely than any
/// paraphrase.
pub fn content_gap(mode: Mode, data: &serde_json::Value, today: Date) -> Option<String> {
    match mode {
        Mode::PreMarket => {
            if has_pre_market_data(data, today) {
                return None;
            }
            let f = pre_market_freshness(data, today);
            if f.is_stale {
                // The case that reads as healthy: every field populated, all of
                // it describing a market day in the past. Only the date tells.
                Some(format!(
                    "stale: payload date={} today={today}{}",
                    f.source_date.as_deref().unwrap_or("unknown"),
                    f.age_days
                        .map(|d| format!(" age={d}d"))
                        .unwrap_or_default(),
                ))
            } else {
                Some(
                    "no signals: high_confidence_signals empty and symbols_analyzed/overall_sentiment absent"
                        .into(),
                )
            }
        }
        Mode::Intraday => {
            if has_intraday_data(data) {
                return None;
            }
            Some(
                quoted(data.get("message"))
                    .unwrap_or_else(|| "no symbols: total_symbols and symbols both empty".into()),
            )
        }
        Mode::Eod => {
            if has_eod_data(data) {
                return None;
            }
            // The placeholder explains itself in key_events — "Market closed;
            // EOD analysis not yet available" is the upstream job's status.
            let events = data
                .get("daily_summary")
                .and_then(|s| s.get("key_events"))
                .and_then(|v| v.as_array())
                .map(|a| {
                    tidy(
                        &a.iter()
                            .filter_map(|e| e.as_str())
                            .collect::<Vec<_>>()
                            .join("; "),
                    )
                })
                .filter(|s| !s.is_empty());
            Some(events.unwrap_or_else(|| {
                "no scorecard: signalBreakdown, totalSignals and symbols_analyzed all empty".into()
            }))
        }
        Mode::Weekly => {
            if has_weekly_data(data) {
                return None;
            }
            let report = data.get("report").unwrap_or(data);
            Some(
                quoted(report.get("message"))
                    .or_else(|| quoted(data.get("message")))
                    .unwrap_or_else(|| "no weekly_overview and no daily_breakdown".into()),
            )
        }
    }
}
