//! TomTom routing call and response parsing.
//!
//! Ports `fetch_route()` from traffic/scripts/run.py:53-66.

use std::fmt;

#[derive(Debug)]
pub enum RouteError {
    NoRoutes,
    MissingTravelTime,
    Http(String),
    Parse(String),
}

impl fmt::Display for RouteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // run.py:65, reproduced exactly — it reaches the user.
            RouteError::NoRoutes => write!(f, "No routes returned by TomTom API"),
            // Python raises KeyError here and main() renders it as
            // `[WARN: traffic unavailable - 'summary']`. The exact text of a
            // KeyError is not worth reproducing; what matters is that this is
            // an error and never a silent zero, which would render as a
            // plausible "0分鐘".
            RouteError::MissingTravelTime => write!(f, "route summary missing travelTimeInSeconds"),
            RouteError::Http(e) => write!(f, "{e}"),
            RouteError::Parse(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for RouteError {}

/// Pull the travel time out of a TomTom `calculateRoute` payload.
pub fn travel_time_seconds(payload: &serde_json::Value) -> Result<i64, RouteError> {
    // `data.get("routes", [])` — absent and empty behave the same (run.py:63).
    let routes = payload.get("routes").and_then(|r| r.as_array());
    let first = match routes {
        Some(list) if !list.is_empty() => &list[0],
        _ => return Err(RouteError::NoRoutes),
    };

    first
        .get("summary")
        .and_then(|s| s.get("travelTimeInSeconds"))
        .and_then(|t| t.as_i64())
        .ok_or(RouteError::MissingTravelTime)
}

/// Render an HTTP status the way Python's urllib does.
///
/// The reason this exists rather than `e.to_string()`: ureq's error Display
/// includes the whole request URL, and the request URL carries `key=<the API
/// key>`. That string is printed to stdout as
/// `[WARN: traffic unavailable - ...]`, which under commute IS the delivered
/// Telegram message — so a 401 would have mailed the key to the user. The
/// Python says `HTTP Error 401: Unauthorized` and nothing else.
///
/// Only the codes TomTom actually returns get a reason phrase; anything else
/// renders bare rather than inventing one.
pub fn status_message(code: u16) -> String {
    let reason = match code {
        400 => Some("Bad Request"),
        401 => Some("Unauthorized"),
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

/// Render a transport failure without echoing the URL.
///
/// ureq's `Transport` Display also names the host and path. The kind alone is
/// what a reader can act on.
pub fn transport_message(kind: &str) -> String {
    format!("TomTom request failed: {kind}")
}

pub const DEFAULT_BASE: &str = "https://api.tomtom.com";

/// Build the request URL. Separate from the call so a test can read it without
/// reaching the network.
///
/// `base` exists so a test can point the whole binary at a local stub, the same
/// seam weather uses (`HKO_BASE_URL` and friends). Production passes None.
pub fn route_url(base: Option<&str>, waypoints: &[String], api_key: &str) -> String {
    let base = base.unwrap_or(DEFAULT_BASE);
    let coords = waypoints.join(":");
    format!("{base}/routing/1/calculateRoute/{coords}/json?key={api_key}&traffic=true")
}

/// Fetch and parse. The 20-second timeout matches run.py:61.
pub fn fetch(base: Option<&str>, waypoints: &[String], api_key: &str) -> Result<i64, RouteError> {
    let url = route_url(base, waypoints, api_key);
    let resp = ureq::get(&url)
        .set("Accept", "application/json")
        .timeout(std::time::Duration::from_secs(20))
        .call()
        // Never `e.to_string()` here — see status_message().
        .map_err(|e| match e {
            ureq::Error::Status(code, _) => RouteError::Http(status_message(code)),
            ureq::Error::Transport(t) => RouteError::Http(transport_message(t.kind().to_string().as_str())),
        })?;

    // ureq 2 hands back 2xx as Ok, and a 204 carries no body — insist on 200
    // rather than trusting the Ok arm (phase ① lessons, section 3).
    if resp.status() != 200 {
        return Err(RouteError::Http(status_message(resp.status())));
    }

    // into_string() + serde_json, matching every other crate here — into_json()
    // needs ureq's `json` feature, which this workspace does not enable.
    let text = resp
        .into_string()
        .map_err(|e| RouteError::Parse(e.to_string()))?;
    let payload: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| RouteError::Parse(e.to_string()))?;
    travel_time_seconds(&payload)
}
