//! Fetching a report from the CCT worker.

pub const DEFAULT_BASE: &str = "https://tft-trading-system.yanggf.workers.dev";

/// The API key, from nullclaw config, defaulting to the public value.
///
/// `yanggf` is not a secret: public/js/config.js assigns it to
/// window.CCT_API_KEY and the worker serves that file to every browser. The
/// default is kept because the key is a routing token, not a credential.
pub fn api_key() -> String {
    let path = std::env::var("CLAW_CONFIG").unwrap_or_else(|_| {
        format!(
            "{}/.nullclaw/config.json",
            std::env::var("HOME").unwrap_or_default()
        )
    });
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|c| {
            c.get("cct")?
                .get("api_key")?
                .as_str()
                .map(str::to_string)
        })
        .unwrap_or_else(|| "yanggf".into())
}

pub fn base() -> String {
    std::env::var("CCT_BASE").unwrap_or_else(|_| DEFAULT_BASE.into())
}

/// GET a report, unwrapping the envelope.
///
/// `None` means "no usable payload", and the reason goes to stderr — stdout is
/// the delivered body plus the contract markers, so a diagnostic there would
/// become part of the message.
/// The CCT API renders a full report before answering.
const TIMEOUT_S: u64 = 30;

pub fn get(path: &str) -> Option<serde_json::Value> {
    let url = format!("{}{path}", base());
    let resp = match claw_core::http::agent(std::time::Duration::from_secs(TIMEOUT_S))
        .get(&url)
        .set("X-API-Key", &api_key())
        .set("User-Agent", "nullclaw-cct/1.0")
        .call()
    {
        Ok(r) => r,
        Err(ureq::Error::Status(code, r)) => {
            let body = r.into_string().unwrap_or_default();
            eprintln!(
                "[WARN: CCT HTTP {code}] {}",
                body.chars().take(120).collect::<String>()
            );
            return None;
        }
        Err(e) => {
            eprintln!("[WARN: CCT unavailable - {}]", e.kind());
            return None;
        }
    };

    let text = resp.into_string().ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&text).ok()?;
    if parsed.get("success").and_then(|v| v.as_bool()) != Some(true) {
        return None;
    }
    let data = parsed.get("data")?.clone();

    // Some routes carry a second envelope inside the payload: weekly serves a
    // DO-cache miss as outer success:true wrapping inner success:false. Test
    // for an explicit false — the other routes omit the key entirely and must
    // not be caught here.
    if data.get("success").and_then(|v| v.as_bool()) == Some(false) {
        eprintln!(
            "[WARN: CCT payload error] {}",
            data.get("error").and_then(|v| v.as_str()).unwrap_or("unknown")
        );
        return None;
    }
    Some(data)
}
