//! Config path + bot-token resolution.
//!
//! JSON is quarantined here: serde_json is used to read the file and the result
//! is immediately reduced to an Option<String>. No JSON value escapes this module.

use std::path::{Path, PathBuf};

pub const CONFIG_ENV: &str = "CLAW_CONFIG";

pub fn default_config_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home).join(".nullclaw/config.json")
}

pub fn resolve_config_path(explicit: Option<&Path>) -> PathBuf {
    if let Some(p) = explicit {
        return p.to_path_buf();
    }
    match std::env::var(CONFIG_ENV) {
        Ok(v) if !v.is_empty() => PathBuf::from(v),
        _ => default_config_path(),
    }
}

/// Returns None on ANY failure — missing file, bad permissions, malformed JSON,
/// missing keys, or an empty token. Never panics, never distinguishes causes:
/// the caller's only question is "do I have a token".
pub fn get_bot_token(account: &str, explicit: Option<&Path>) -> Option<String> {
    let path = resolve_config_path(explicit);
    let body = std::fs::read_to_string(path).ok()?;
    let cfg: serde_json::Value = serde_json::from_str(&body).ok()?;
    let telegram = cfg.get("channels")?.get("telegram")?;

    let account_token = telegram
        .get("accounts")
        .and_then(|a| a.get(account))
        .and_then(|a| a.get("bot_token"))
        .and_then(|t| t.as_str())
        .filter(|t| !t.is_empty());
    if let Some(t) = account_token {
        return Some(t.to_string());
    }

    telegram
        .get("botToken")
        .and_then(|t| t.as_str())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_string())
}
