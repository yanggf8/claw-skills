//! PizzINT dashboard adapter. JSON stops here.

use std::time::Duration;

/// `overall_index` as Python actually uses it. Python does exactly two things
/// with this value: it renders it into an f-string, and it compares `== 0`.
/// So that is what this carries — nothing else.
///
/// Getting this wrong in EITHER direction flips the no-data path, which flips
/// the skill status, which nullclaw escalates to last_status=error plus a retry:
///   * narrowing to `Option<i64>` turns a float index into the -1 sentinel;
///   * assuming "non-integer means non-zero" misses `0.0`, `-0.0` and `false`,
///     all of which satisfy Python's `== 0` (in Python, `False == 0` is True).
#[derive(Debug, Clone, PartialEq)]
pub enum RawIndex {
    /// Absent or JSON null — Python's `raw_index is None`.
    Missing,
    Present {
        /// What Python's f-string would print.
        rendered: String,
        /// What Python's `raw_index == 0` would answer.
        is_zero: bool,
    },
}

/// Python renders booleans capitalised and floats with a decimal point. serde's
/// own rendering matches for numbers and strings but not for booleans.
fn render_like_python(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Bool(b) => if *b { "True".into() } else { "False".into() },
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Python's `== 0`: true for 0, 0.0, -0.0 and False. A string is never equal to
/// an int in Python, so "0" is NOT zero.
fn is_python_zero(v: &serde_json::Value) -> bool {
    match v {
        serde_json::Value::Number(n) => n.as_f64().map(|f| f == 0.0).unwrap_or(false),
        serde_json::Value::Bool(b) => !*b,
        _ => false,
    }
}

pub struct Snapshot {
    pub level: String,
    pub raw_index: RawIndex,
    /// One entry per place: `true` when `current_popularity` is JSON null or
    /// absent. Python tests `is None` ONLY — a non-numeric value such as the
    /// string "x" is NOT null there, so parsing to `Option<f64>` and treating a
    /// parse failure as null would flip a live `ok` into `degraded`.
    pub popularity_is_null: Vec<bool>,
    pub timestamp: Option<String>,
}

pub const DEFAULT_URL: &str = "https://pizzint.watch/api/dashboard-data";
const TIMEOUT_S: u64 = 20;

pub fn fetch(base_url: Option<&str>) -> Result<Snapshot, String> {
    let url = base_url.unwrap_or(DEFAULT_URL);
    let body = claw_core::http::agent(Duration::from_secs(TIMEOUT_S))
        .get(url)
        .set("Accept", "application/json")
        .set("User-Agent", "nullclaw/1.0")
        .call()
        .map_err(|e| e.to_string())?
        .into_string()
        .map_err(|e| e.to_string())?;
    parse(&body)
}

/// A payload that is not a JSON object is rejected HERE, so it routes to the
/// same degraded path as a fetch failure. Python would have thrown later,
/// outside the fetch handler, and exited hard — recorded as an intentional
/// difference.
pub fn parse(body: &str) -> Result<Snapshot, String> {
    let v: serde_json::Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    let obj = v.as_object().ok_or_else(|| "payload is not a JSON object".to_string())?;

    // Python is `data.get("defcon_level", "?")`, so "?" appears ONLY when the key
    // is absent. A null level renders as "None", a bool as "True"/"False".
    let level = match obj.get("defcon_level") {
        None => "?".to_string(),
        Some(serde_json::Value::Null) => "None".to_string(),
        Some(v) => render_like_python(v),
    };
    let raw_index = match obj.get("overall_index") {
        None | Some(serde_json::Value::Null) => RawIndex::Missing,
        Some(v) => RawIndex::Present {
            rendered: render_like_python(v),
            is_zero: is_python_zero(v),
        },
    };

    // Nullness only — the values are never used, and matching Python means
    // asking "is it JSON null / absent", not "does it parse as a number".
    let popularity_is_null = obj
        .get("data")
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .map(|p| {
                    matches!(
                        p.get("current_popularity"),
                        None | Some(serde_json::Value::Null)
                    )
                })
                .collect()
        })
        .unwrap_or_default();

    let timestamp = obj.get("timestamp").and_then(|t| t.as_str()).map(String::from);

    Ok(Snapshot { level, raw_index, popularity_is_null, timestamp })
}
