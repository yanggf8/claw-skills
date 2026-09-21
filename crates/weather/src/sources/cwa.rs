//! CWA (Taiwan Central Weather Administration) adapter.
//!
//! Oracle: weather/scripts/run.py `fetch_cwa_weather` + `format_cwa_location`
//! and the `records = cwa_data.get("records", {}).get("location", []) or []`
//! extraction (lines 107–115, 170–205, 282).
//! Timeout 8s (B10). Null location key → empty list, not error (B9).
//! Slot selection: nearest startTime to now in UTC+8 (B15).

use std::time::Duration;

use jiff::{civil::DateTime, tz::TimeZone, Timestamp, Zoned};

use super::Row;

pub const DEFAULT_BASE: &str =
    "https://opendata.cwa.gov.tw/api/v1/rest/datastore/F-C0032-001";
const TIMEOUT_S: u64 = 8;

/// One CWA location record (the map value keyed by locationName in Python).
#[derive(Debug, Clone)]
pub struct CwaLocation {
    pub location_name: String,
    pub elements: Vec<CwaElement>,
}

#[derive(Debug, Clone)]
pub struct CwaElement {
    pub name: String,
    pub times: Vec<CwaTimeSlot>,
}

#[derive(Debug, Clone)]
pub struct CwaTimeSlot {
    pub start_time: String,
    pub parameter_name: String,
}

pub fn fetch(
    base_url: Option<&str>,
    locations: &[String],
    api_key: &str,
) -> Result<String, String> {
    let base = base_url.unwrap_or(DEFAULT_BASE);
    let joined = locations
        .iter()
        .map(|l| urlencoding_quote(l))
        .collect::<Vec<_>>()
        .join(",");
    let url = if base.contains('?') {
        format!("{base}&Authorization={api_key}&locationName={joined}")
    } else {
        format!("{base}?Authorization={api_key}&locationName={joined}")
    };
    claw_core::http::agent(Duration::from_secs(TIMEOUT_S))
        .get(&url)
        .set("Accept", "application/json")
        .call()
        .map_err(|e| e.to_string())?
        .into_string()
        .map_err(|e| e.to_string())
}

/// Python's `urllib.parse.quote` default (safe='/').
fn urlencoding_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        match *b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(*b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Extract `records.location` — null or missing → empty vec (B9).
pub fn records(body: &str) -> Result<Vec<CwaLocation>, String> {
    let v: serde_json::Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    let obj = v
        .as_object()
        .ok_or_else(|| "CWA payload is not a JSON object".to_string())?;

    // records = data.get("records", {})
    let records = match obj.get("records") {
        None | Some(serde_json::Value::Null) => return Ok(Vec::new()),
        Some(serde_json::Value::Object(m)) => m,
        Some(_) => return Ok(Vec::new()),
    };

    // location = records.get("location", []) or []
    let loc_arr = match records.get("location") {
        None | Some(serde_json::Value::Null) => return Ok(Vec::new()),
        Some(serde_json::Value::Array(a)) => a,
        Some(_) => return Ok(Vec::new()),
    };

    let mut out = Vec::with_capacity(loc_arr.len());
    for loc in loc_arr {
        let location_name = loc
            .get("locationName")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let elements = match loc.get("weatherElement") {
            Some(serde_json::Value::Array(els)) => els
                .iter()
                .map(|el| {
                    let name = el
                        .get("elementName")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string();
                    let times = match el.get("time") {
                        Some(serde_json::Value::Array(ts)) => ts
                            .iter()
                            .map(|tv| CwaTimeSlot {
                                start_time: tv
                                    .get("startTime")
                                    .and_then(|x| x.as_str())
                                    .unwrap_or("")
                                    .to_string(),
                                parameter_name: tv
                                    .get("parameter")
                                    .and_then(|p| p.get("parameterName"))
                                    .and_then(|x| x.as_str())
                                    .unwrap_or("")
                                    .to_string(),
                            })
                            .collect(),
                        _ => Vec::new(),
                    };
                    CwaElement { name, times }
                })
                .collect(),
            _ => Vec::new(),
        };
        out.push(CwaLocation {
            location_name,
            elements,
        });
    }
    Ok(out)
}

/// Format one CWA location. Always returns a row (B7: CWA appends unconditionally).
pub fn format(loc_name: &str, loc_data: &CwaLocation) -> (String, Option<Row>) {
    let by_name = |name: &str| -> &[CwaTimeSlot] {
        loc_data
            .elements
            .iter()
            .find(|e| e.name == name)
            .map(|e| e.times.as_slice())
            .unwrap_or(&[])
    };

    let wx_slots = by_name("Wx");
    let best_idx = pick_nearest_slot(wx_slots);

    let val_at = |name: &str| -> String {
        let times = by_name(name);
        if best_idx < times.len() {
            times[best_idx].parameter_name.clone()
        } else {
            String::new()
        }
    };

    let wx = val_at("Wx");
    let min_t = val_at("MinT");
    let max_t = val_at("MaxT");
    let pop = val_at("PoP");

    let mut line = format!("🌤 {loc_name}：{wx}，低溫{min_t}°C / 高溫{max_t}°C");
    // B14: "0" is truthy.
    if !pop.is_empty() {
        line.push_str(&format!("，降雨機率{pop}%"));
    }

    let row = Row {
        location: loc_name.to_string(),
        wx,
        min_t,
        max_t,
        pop,
    };
    (line, Some(row))
}

/// Pick the Wx slot whose startTime is nearest to now in UTC+8 (B15).
fn pick_nearest_slot(wx_slots: &[CwaTimeSlot]) -> usize {
    let now_ts = Timestamp::now();
    let mut best_idx = 0usize;
    let mut best_delta: Option<i64> = None;

    for (i, tv) in wx_slots.iter().enumerate() {
        let Some(z) = parse_cwa_start(&tv.start_time) else {
            continue;
        };
        let slot_ts = z.timestamp();
        // abs((dt - now).total_seconds()) as integer nanoseconds / 1e9
        let delta_ns = (slot_ts.as_nanosecond() - now_ts.as_nanosecond()).unsigned_abs();
        let delta_s = (delta_ns / 1_000_000_000) as i64;
        if best_delta.is_none() || delta_s < best_delta.unwrap() {
            best_delta = Some(delta_s);
            best_idx = i;
        }
    }
    best_idx
}

/// Parse `"%Y-%m-%d %H:%M:%S"` as UTC+8 civil time (Python strptime + replace tzinfo).
fn parse_cwa_start(s: &str) -> Option<Zoned> {
    let dt: DateTime = s.parse().ok()?;
    let tz = TimeZone::fixed(jiff::tz::offset(8));
    dt.to_zoned(tz).ok()
}
