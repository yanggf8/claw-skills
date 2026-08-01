//! Open-Meteo fallback adapter.
//!
//! Oracle: weather/scripts/run.py `fetch_open_meteo` + `format_open_meteo`
//! (+ WMO_TC / TW_COORDS consumers) at lines 120–167.
//! Timeout 8s (B10). Temps use Python half-to-even round (B6).
//! Rain "0" is truthy (B14). Missing arrays → WARN + no row (B7).

use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Duration;

use super::Row;

pub const DEFAULT_BASE: &str = "https://api.open-meteo.com/v1/forecast";
const TIMEOUT_S: u64 = 8;

/// Parsed Open-Meteo daily block (only fields format_open_meteo uses).
#[derive(Debug, Clone)]
pub struct OmData {
    pub weather_codes: Vec<i64>,
    pub max_temps: Vec<f64>,
    pub min_temps: Vec<f64>,
    pub pops: Vec<f64>,
}

/// Taiwan county/city centroids for Open-Meteo fallback (lat, lon).
/// Oracle: run.py TW_COORDS.
pub fn tw_coords() -> &'static HashMap<&'static str, (f64, f64)> {
    static MAP: OnceLock<HashMap<&'static str, (f64, f64)>> = OnceLock::new();
    MAP.get_or_init(|| {
        HashMap::from([
            ("臺北市", (25.0330, 121.5654)),
            ("台北市", (25.0330, 121.5654)),
            ("新北市", (25.0169, 121.4628)),
            ("桃園市", (24.9937, 121.3010)),
            ("臺中市", (24.1477, 120.6736)),
            ("台中市", (24.1477, 120.6736)),
            ("臺南市", (22.9999, 120.2270)),
            ("台南市", (22.9999, 120.2270)),
            ("高雄市", (22.6273, 120.3014)),
            ("基隆市", (25.1276, 121.7392)),
            ("新竹市", (24.8138, 120.9675)),
            ("新竹縣", (24.8387, 121.0177)),
            ("苗栗縣", (24.5602, 120.8214)),
            ("彰化縣", (24.0518, 120.5161)),
            ("南投縣", (23.9609, 120.9719)),
            ("雲林縣", (23.7092, 120.4313)),
            ("嘉義市", (23.4801, 120.4491)),
            ("嘉義縣", (23.4518, 120.2555)),
            ("屏東縣", (22.5519, 120.5487)),
            ("宜蘭縣", (24.7021, 121.7378)),
            ("花蓮縣", (23.9871, 121.6015)),
            ("臺東縣", (22.7583, 121.1444)),
            ("台東縣", (22.7583, 121.1444)),
            ("澎湖縣", (23.5712, 119.5793)),
            ("金門縣", (24.4321, 118.3171)),
            ("連江縣", (26.1608, 119.9286)),
        ])
    })
}

/// WMO weather code → Traditional Chinese. Oracle: run.py WMO_TC.
fn wmo_tc(code: i64) -> &'static str {
    match code {
        0 => "晴朗",
        1 => "大致晴朗",
        2 => "局部多雲",
        3 => "陰天",
        45 => "霧",
        48 => "凍霧",
        51 | 53 => "毛毛雨",
        55 => "強毛毛雨",
        56 | 57 => "凍毛毛雨",
        61 => "小雨",
        63 => "中雨",
        65 => "大雨",
        66 | 67 => "凍雨",
        71 => "小雪",
        73 => "中雪",
        75 => "大雪",
        77 => "雪粒",
        80 => "短暫陣雨",
        81 => "陣雨",
        82 => "強陣雨",
        85 => "短暫陣雪",
        86 => "陣雪",
        95 => "雷雨",
        96 => "雷雨夾冰雹",
        99 => "強雷雨夾冰雹",
        _ => "",
    }
}

pub fn fetch(base_url: Option<&str>, lat: f64, lon: f64) -> Result<OmData, String> {
    let base = base_url.unwrap_or(DEFAULT_BASE);
    let url = if base.contains('?') {
        // test seam already carries query — just use it
        base.to_string()
    } else {
        format!(
            "{base}?latitude={lat}&longitude={lon}\
             &daily=temperature_2m_max,temperature_2m_min,precipitation_probability_max,weather_code\
             &timezone=Asia%2FTaipei&forecast_days=1"
        )
    };
    let body = claw_core::http::agent(Duration::from_secs(TIMEOUT_S))
        .get(&url)
        .set("Accept", "application/json")
        .call()
        .map_err(|e| e.to_string())?
        .into_string()
        .map_err(|e| e.to_string())?;
    parse(&body)
}

pub fn parse(body: &str) -> Result<OmData, String> {
    let v: serde_json::Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    let obj = v
        .as_object()
        .ok_or_else(|| "Open-Meteo payload is not a JSON object".to_string())?;

    // daily = data.get("daily", {})
    let daily = match obj.get("daily") {
        None | Some(serde_json::Value::Null) => {
            return Ok(OmData {
                weather_codes: vec![],
                max_temps: vec![],
                min_temps: vec![],
                pops: vec![],
            });
        }
        Some(serde_json::Value::Object(m)) => m,
        Some(_) => {
            return Ok(OmData {
                weather_codes: vec![],
                max_temps: vec![],
                min_temps: vec![],
                pops: vec![],
            });
        }
    };

    // The oracle reads exactly `weather_code` (run.py:134). Accepting a second
    // spelling would parse a payload the Python treats as missing codes, which
    // routes to the WARN path and yields no row — a real divergence.
    let weather_codes = num_array_i64(daily.get("weather_code"));
    let max_temps = num_array_f64(daily.get("temperature_2m_max"));
    let min_temps = num_array_f64(daily.get("temperature_2m_min"));
    let pops = num_array_f64(daily.get("precipitation_probability_max"));

    Ok(OmData {
        weather_codes,
        max_temps,
        min_temps,
        pops,
    })
}

/// `.get(key, []) or []` — null / non-array → empty.
fn num_array_f64(v: Option<&serde_json::Value>) -> Vec<f64> {
    match v {
        Some(serde_json::Value::Array(a)) => a.iter().filter_map(|x| x.as_f64()).collect(),
        _ => Vec::new(),
    }
}

fn num_array_i64(v: Option<&serde_json::Value>) -> Vec<i64> {
    match v {
        Some(serde_json::Value::Array(a)) => a
            .iter()
            .filter_map(|x| {
                x.as_i64()
                    .or_else(|| x.as_f64().map(|f| f as i64))
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Python 3 `round()` — half to even (banker's rounding). Do NOT use `f64::round`.
pub fn round_like_python(x: f64) -> i64 {
    let floored = x.floor();
    let diff = x - floored;
    if diff < 0.5 {
        floored as i64
    } else if diff > 0.5 {
        (floored + 1.0) as i64
    } else {
        // exactly halfway: pick the even integer
        let n = floored as i64;
        if n % 2 == 0 {
            n
        } else {
            n + 1
        }
    }
}

pub fn format(loc_name: &str, data: &OmData) -> (String, Option<Row>) {
    // Python: if not codes or not maxs or not mins
    if data.weather_codes.is_empty() || data.max_temps.is_empty() || data.min_temps.is_empty() {
        return (
            format!("[WARN: Open-Meteo forecast unavailable for {loc_name}]"),
            None,
        );
    }
    let wx = wmo_tc(data.weather_codes[0]).to_string();
    let max_t = round_like_python(data.max_temps[0]).to_string();
    let min_t = round_like_python(data.min_temps[0]).to_string();
    // pop = str(int(pops[0])) if pops else ""
    let pop = if data.pops.is_empty() {
        String::new()
    } else {
        // Python int() truncates toward zero on floats.
        (data.pops[0] as i64).to_string()
    };

    let mut line = format!("🌤 {loc_name}：{wx}，低溫{min_t}°C / 高溫{max_t}°C");
    // B14: "0" is truthy.
    if !pop.is_empty() {
        line.push_str(&format!("，降雨機率{pop}%"));
    }
    line.push_str("（備援）");

    let row = Row {
        location: loc_name.to_string(),
        wx,
        min_t,
        max_t,
        pop,
    };
    (line, Some(row))
}

/// Per-location Open-Meteo loop. Oracle: `open_meteo_for_locations`.
pub fn for_locations(locations: &[String]) -> (Vec<String>, Vec<Row>) {
    let mut lines = Vec::new();
    let mut weather_data = Vec::new();
    for loc in locations {
        let Some(&(lat, lon)) = tw_coords().get(loc.as_str()) else {
            lines.push(format!(
                "[WARN: weather unavailable - no fallback coordinates for '{loc}']"
            ));
            continue;
        };
        match fetch(None, lat, lon) {
            Ok(data) => {
                let (line, wd) = format(loc, &data);
                lines.push(line);
                if let Some(row) = wd {
                    weather_data.push(row);
                }
            }
            Err(e) => {
                lines.push(format!("[WARN: Open-Meteo unavailable for {loc} - {e}]"));
            }
        }
    }
    (lines, weather_data)
}
