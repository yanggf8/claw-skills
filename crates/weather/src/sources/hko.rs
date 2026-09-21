//! HKO (Hong Kong Observatory) adapter.
//!
//! Oracle: weather/scripts/run.py `fetch_hko_forecast` + `format_hko` (lines 83–103).
//! Timeouts: 20s (B10). Line/row always say 香港, never loc_name (B3).
//! Rain: 降雨概率{psr} with no percent sign (B5). Empty forecast → WARN + no row (B7).

use std::time::Duration;

use super::Row;

pub const DEFAULT_URL: &str =
    "https://data.weather.gov.hk/weatherAPI/opendata/weather.php?dataType=fnd&lang=tc";
const TIMEOUT_S: u64 = 20;

/// Parsed HKO 9-day forecast payload (only the fields format_hko uses).
#[derive(Debug, Clone)]
pub struct HkoData {
    pub forecasts: Vec<HkoForecast>,
}

#[derive(Debug, Clone)]
pub struct HkoForecast {
    pub wx: String,
    /// Already stringified like Python's `str(...)`; `"?"` when value is absent.
    pub min_t: String,
    pub max_t: String,
    pub psr: String,
}

pub fn fetch(base_url: Option<&str>) -> Result<HkoData, String> {
    let url = base_url.unwrap_or(DEFAULT_URL);
    let body = claw_core::http::agent(Duration::from_secs(TIMEOUT_S))
        .get(url)
        .set("Accept", "application/json")
        .call()
        .map_err(|e| e.to_string())?
        .into_string()
        .map_err(|e| e.to_string())?;
    parse(&body)
}

pub fn parse(body: &str) -> Result<HkoData, String> {
    let v: serde_json::Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    let obj = v
        .as_object()
        .ok_or_else(|| "HKO payload is not a JSON object".to_string())?;

    // Python: data.get("weatherForecast", []) — null is falsy → empty.
    let forecasts = match obj.get("weatherForecast") {
        None | Some(serde_json::Value::Null) => Vec::new(),
        Some(serde_json::Value::Array(arr)) => arr
            .iter()
            .map(|f| {
                let wx = f
                    .get("forecastWeather")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                let min_t = temp_value(f.get("forecastMintemp"));
                let max_t = temp_value(f.get("forecastMaxtemp"));
                let psr = f
                    .get("PSR")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                HkoForecast {
                    wx,
                    min_t,
                    max_t,
                    psr,
                }
            })
            .collect(),
        Some(_) => Vec::new(),
    };

    Ok(HkoData { forecasts })
}

/// Python: `f.get("forecastMintemp", {}).get("value", "?")` then `str(...)`.
fn temp_value(obj: Option<&serde_json::Value>) -> String {
    let Some(obj) = obj else {
        return "?".to_string();
    };
    if obj.is_null() {
        // Python would crash on None.get; treat as missing for a clean WARN path.
        return "?".to_string();
    }
    match obj.get("value") {
        None => "?".to_string(),
        Some(serde_json::Value::Null) => "None".to_string(),
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Number(n)) => n.to_string(),
        Some(serde_json::Value::Bool(b)) => {
            if *b {
                "True".to_string()
            } else {
                "False".to_string()
            }
        }
        Some(other) => other.to_string(),
    }
}

/// Format one HKO line. `loc_name` is used only in the empty-forecast WARN message
/// (B3: success line and row always say 香港).
pub fn format(loc_name: &str, data: &HkoData) -> (String, Option<Row>) {
    if data.forecasts.is_empty() {
        return (
            format!("[WARN: HKO forecast unavailable for {loc_name}]"),
            None,
        );
    }
    let f = &data.forecasts[0];
    let mut line = format!(
        "🌤 香港：{}，低溫{}°C / 高溫{}°C",
        f.wx, f.min_t, f.max_t
    );
    // B14: non-empty string is truthy — including values that are not "high".
    if !f.psr.is_empty() {
        // B5: no percent sign — PSR is qualitative (e.g. 高).
        line.push_str(&format!("，降雨概率{}", f.psr));
    }
    let row = Row {
        location: "香港".to_string(),
        wx: f.wx.clone(),
        min_t: f.min_t.clone(),
        max_t: f.max_t.clone(),
        pop: f.psr.clone(),
    };
    (line, Some(row))
}
