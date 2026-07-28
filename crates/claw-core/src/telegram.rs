//! Bounded-retry Telegram send.
//!
//! Ported from lib/telegram.py. The retry policy is deliberate and narrow:
//! only 429 and 5xx plus transport failures are retried. 408 is NOT retryable.
//! HTTP 200 is success without reading the response body.
//!
//! JSON appears only as the request payload built here; no JSON value escapes.

use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::config::get_bot_token;

pub const PER_ATTEMPT_TIMEOUT_S: f64 = 15.0;
pub const DEFAULT_DEADLINE_S: f64 = 30.0;
pub const BACKOFFS_S: [f64; 2] = [2.0, 5.0];
pub const DEFAULT_BASE_URL: &str = "https://api.telegram.org";

pub struct SendOptions {
    pub account: String,
    pub config_path: Option<PathBuf>,
    pub deadline_s: Option<f64>,
    pub parse_mode: Option<String>,
    /// Test seam only. Defaults to the real host.
    pub base_url: Option<String>,
}

impl Default for SendOptions {
    fn default() -> Self {
        Self {
            account: "main".into(),
            config_path: None,
            deadline_s: None,
            parse_mode: Some("Markdown".into()),
            base_url: None,
        }
    }
}

fn log(msg: &str) {
    let mut err = std::io::stderr();
    let _ = writeln!(err, "[telegram] {msg}");
    let _ = err.flush();
}

fn is_retryable_http(code: u16) -> bool {
    code == 429 || (500..600).contains(&code)
}

pub fn send(chat_id: &str, text: &str, opts: &SendOptions) -> bool {
    // No token: silent false. The [delivery] diagnostic is the caller's job.
    let Some(token) = get_bot_token(&opts.account, opts.config_path.as_deref()) else {
        return false;
    };

    let base = opts.base_url.as_deref().unwrap_or(DEFAULT_BASE_URL);
    let url = format!("{}/bot{}/sendMessage", base.trim_end_matches('/'), token);

    let mut payload = serde_json::Map::new();
    payload.insert("chat_id".into(), serde_json::Value::from(chat_id));
    payload.insert("text".into(), serde_json::Value::from(text));
    payload.insert("disable_web_page_preview".into(), serde_json::Value::Bool(true));
    if let Some(pm) = &opts.parse_mode {
        payload.insert("parse_mode".into(), serde_json::Value::from(pm.as_str()));
    }
    let body = serde_json::Value::Object(payload).to_string();

    let budget = opts.deadline_s.unwrap_or(DEFAULT_DEADLINE_S);
    if budget <= 0.0 {
        log(&format!("deadline already exhausted (budget={budget:.1}s); skipping send"));
        return false;
    }

    // Budget clock starts here — after token lookup and payload construction,
    // matching Python.
    let start = Instant::now();
    let max_attempts = 1 + BACKOFFS_S.len();

    for attempt in 1..=max_attempts {
        let elapsed = start.elapsed().as_secs_f64();
        let remaining = budget - elapsed;
        if remaining <= 0.0 {
            log(&format!("deadline exhausted before attempt {attempt}/{max_attempts}"));
            return false;
        }
        let per_attempt = PER_ATTEMPT_TIMEOUT_S.min(remaining);

        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs_f64(per_attempt))
            .build();
        let result = agent
            .post(&url)
            .set("Content-Type", "application/json")
            .send_string(&body);

        match result {
            Ok(resp) => {
                if resp.status() == 200 {
                    return true;
                }
                log(&format!(
                    "unexpected status {} on attempt {attempt}/{max_attempts}",
                    resp.status()
                ));
                return false;
            }
            Err(ureq::Error::Status(code, resp)) => {
                // Python appends up to 200 chars of the response body. That text
                // is how a bad chat_id, a revoked token, or broken markup gets
                // diagnosed in production — dropping it degrades ops for no gain.
                let body = resp
                    .into_string()
                    .unwrap_or_default()
                    .chars()
                    .take(200)
                    .collect::<String>();
                if !is_retryable_http(code) {
                    log(&format!(
                        "permanent HTTP {code} on attempt {attempt}/{max_attempts}: {body}"
                    ));
                    return false;
                }
                let left = budget - start.elapsed().as_secs_f64();
                log(&format!(
                    "attempt {attempt}/{max_attempts} got HTTP {code} (remaining={left:.1}s): {body}"
                ));
            }
            Err(ureq::Error::Transport(t)) => {
                let left = budget - start.elapsed().as_secs_f64();
                log(&format!(
                    "attempt {attempt}/{max_attempts} got transport error (remaining={left:.1}s): {t}"
                ));
            }
        }

        if attempt >= max_attempts {
            break;
        }
        let backoff = BACKOFFS_S[attempt - 1];
        if backoff > 0.0 {
            let remaining_budget = (budget - start.elapsed().as_secs_f64()).max(0.0);
            if remaining_budget <= 0.0 {
                log("no time left for backoff; giving up");
                return false;
            }
            std::thread::sleep(Duration::from_secs_f64(backoff.min(remaining_budget)));
        }
    }

    log(&format!("all {max_attempts} attempts failed"));
    false
}
