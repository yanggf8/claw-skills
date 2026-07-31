//! Fetching and parsing the two upstreams.

use crate::quote::{Quote, Source};
use std::fmt;

pub const TWSE_DEFAULT_BASE: &str = "https://mis.twse.com.tw";
pub const YAHOO_DEFAULT_BASE: &str = "https://query1.finance.yahoo.com";
const TIMEOUT_S: u64 = 15;
const UA: &str = "nullclaw/1.0";

#[derive(Debug)]
pub enum SourceError {
    NoData,
    Http(String),
    Parse(String),
}

impl fmt::Display for SourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SourceError::NoData => write!(f, "no quote in response"),
            SourceError::Http(e) => write!(f, "{e}"),
            SourceError::Parse(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for SourceError {}

/// Render an HTTP status without echoing the request URL.
///
/// Same rule as the traffic skill, for the same reason: these strings are
/// printed into the delivered message, and a URL can carry a token. TWSE's does
/// not today, but the habit is what keeps that true.
fn status_message(code: u16) -> String {
    let reason = match code {
        400 => Some("Bad Request"),
        403 => Some("Forbidden"),
        404 => Some("Not Found"),
        429 => Some("Too Many Requests"),
        500 => Some("Internal Server Error"),
        502 => Some("Bad Gateway"),
        503 => Some("Service Unavailable"),
        504 => Some("Gateway Timeout"),
        _ => None,
    };
    match reason {
        Some(r) => format!("HTTP Error {code}: {r}"),
        None => format!("HTTP Error {code}"),
    }
}

fn get_json(url: &str, referer: Option<&str>) -> Result<serde_json::Value, SourceError> {
    let mut req = ureq::get(url)
        .set("Accept", "application/json")
        .set("User-Agent", UA)
        .timeout(std::time::Duration::from_secs(TIMEOUT_S));
    if let Some(r) = referer {
        req = req.set("Referer", r);
    }
    let resp = req.call().map_err(|e| match e {
        ureq::Error::Status(code, _) => SourceError::Http(status_message(code)),
        ureq::Error::Transport(t) => SourceError::Http(format!("request failed: {}", t.kind())),
    })?;
    if resp.status() != 200 {
        return Err(SourceError::Http(status_message(resp.status())));
    }
    let text = resp
        .into_string()
        .map_err(|e| SourceError::Parse(e.to_string()))?;
    serde_json::from_str(&text).map_err(|e| SourceError::Parse(e.to_string()))
}

// ── TWSE ─────────────────────────────────────────────────────────────────────

/// `20260731` + `13:33:00` → `2026-07-31 13:33:00`.
///
/// The Python interpolated `d` straight into the message, so a person read
/// "，20260731 13:33:00". That is the wire format reaching the page; nobody
/// picked it. Anything that does not look like eight digits is passed through
/// rather than mangled.
fn twse_stamp(date: Option<&str>, time: Option<&str>) -> Option<String> {
    let pretty = date.map(|d| {
        if d.len() == 8 && d.chars().all(|c| c.is_ascii_digit()) {
            format!("{}-{}-{}", &d[0..4], &d[4..6], &d[6..8])
        } else {
            d.to_string()
        }
    });
    match (pretty, time) {
        (Some(d), Some(t)) => Some(format!("{d} {t}")),
        (Some(d), None) => Some(d),
        (None, Some(t)) => Some(t.to_string()),
        (None, None) => None,
    }
}

fn str_field<'a>(o: &'a serde_json::Value, k: &str) -> Option<&'a str> {
    o.get(k).and_then(|v| v.as_str()).filter(|s| !s.is_empty())
}

pub fn parse_twse(payload: &serde_json::Value) -> Result<Quote, SourceError> {
    let first = payload
        .get("msgArray")
        .and_then(|a| a.as_array())
        .and_then(|a| a.first())
        .ok_or(SourceError::NoData)?;

    let price = str_field(first, "z").unwrap_or("-").to_string();
    Ok(Quote {
        name: str_field(first, "n").unwrap_or("?").to_string(),
        price_num: price.parse::<f64>().ok(),
        price,
        prev: str_field(first, "y").and_then(|s| s.parse().ok()),
        high: str_field(first, "h").map(str::to_string),
        low: str_field(first, "l").map(str::to_string),
        stamp: twse_stamp(str_field(first, "d"), str_field(first, "t")),
        source: Source::Twse,
    })
}

pub fn fetch_twse(base: Option<&str>, ex_ch: &str) -> Result<Quote, SourceError> {
    let base = base.unwrap_or(TWSE_DEFAULT_BASE);
    let url = format!("{base}/stock/api/getStockInfo.jsp?ex_ch={ex_ch}&json=1");
    parse_twse(&get_json(&url, None)?)
}

// ── Yahoo ────────────────────────────────────────────────────────────────────

fn num_field(o: &serde_json::Value, k: &str) -> Option<f64> {
    o.get(k).and_then(|v| v.as_f64())
}

/// Render a Unix timestamp in the exchange's own timezone.
///
/// A Hong Kong close shown on the host's clock is a quote on the wrong day
/// whenever the host is not in Hong Kong, which here it is not.
fn yahoo_stamp(meta: &serde_json::Value) -> Option<String> {
    let secs = meta.get("regularMarketTime")?.as_i64()?;
    let tz = meta
        .get("exchangeTimezoneName")
        .and_then(|v| v.as_str())
        .unwrap_or("UTC");
    let ts = jiff::Timestamp::from_second(secs).ok()?;
    let zoned = ts.in_tz(tz).ok()?;
    Some(zoned.strftime("%Y-%m-%d %H:%M:%S").to_string())
}

pub fn parse_yahoo(payload: &serde_json::Value, name: &str) -> Result<Quote, SourceError> {
    let meta = payload
        .get("chart")
        .and_then(|c| c.get("result"))
        .and_then(|r| r.as_array())
        .and_then(|a| a.first())
        .and_then(|r| r.get("meta"))
        .ok_or(SourceError::NoData)?;

    let price_num = num_field(meta, "regularMarketPrice");

    // `previousClose or chartPreviousClose` in the Python. A null or zero
    // previous close falls through, because neither is a usable divisor. The
    // live ^HSI payload omits previousClose entirely, so this is the branch
    // that actually runs.
    let prev = num_field(meta, "previousClose")
        .filter(|v| *v != 0.0)
        .or_else(|| num_field(meta, "chartPreviousClose"));

    Ok(Quote {
        name: name.to_string(),
        price: price_num
            .map(|p| format!("{p}"))
            .unwrap_or_else(|| "?".into()),
        price_num,
        prev,
        high: num_field(meta, "regularMarketDayHigh").map(|v| format!("{v}")),
        low: num_field(meta, "regularMarketDayLow").map(|v| format!("{v}")),
        stamp: yahoo_stamp(meta),
        source: Source::Yahoo,
    })
}

pub fn fetch_yahoo(base: Option<&str>, symbol: &str, name: &str) -> Result<Quote, SourceError> {
    let base = base.unwrap_or(YAHOO_DEFAULT_BASE);
    let url = format!("{base}/v8/finance/chart/{symbol}?interval=1d&range=5d");
    parse_yahoo(&get_json(&url, None)?, name)
}
