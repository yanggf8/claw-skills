//! Whether a payload carries real analysis, per mode.

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
