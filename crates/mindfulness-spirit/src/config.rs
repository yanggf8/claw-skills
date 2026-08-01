//! What the operator configures.

use std::path::PathBuf;

/// The column to write for, when the config does not name one.
///
/// A default, not a constant: which season is running is operator state.
/// Hard-coding it is what turned a finished season into six weeks of silent
/// weekly failure — `plans next` returned nothing, `prepare` answered
/// not-found, and every Friday's run and its retry died the same way.
pub const DEFAULT_COLUMN_SLUG: &str = "machine-and-cushion";

pub const SKILL_NAME: &str = "mindfulness-spirit";
/// Read from the persona's publish history, which spans both columns.
pub const HISTORY_LIMIT: u32 = 8;

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub persona_slug: String,
    pub column_slug: String,
    /// Retained for operator visibility only. Publishing is decided by the
    /// column's `delivery_target`, so that there is one routing source of
    /// truth rather than two that can disagree.
    pub publish: bool,
    pub main_image_url: Option<String>,
}

pub fn config_path() -> PathBuf {
    home().join(".nullclaw/config.json")
}

pub fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
}

/// A missing or unreadable config is not fatal here — the persona check below
/// is what decides that — so it reads as an empty object and the caller
/// produces one clear error instead of two vague ones.
pub fn load_config(warn: &mut impl std::io::Write) -> serde_json::Value {
    let path = config_path();
    let Ok(text) = std::fs::read_to_string(&path) else {
        return serde_json::json!({});
    };
    match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => {
            let _ = writeln!(warn, "Warning: could not read {}: {e}", path.display());
            serde_json::json!({})
        }
    }
}

pub fn load_settings(warn: &mut impl std::io::Write) -> Result<Settings, String> {
    settings_from(&load_config(warn))
}

pub fn settings_from(config: &serde_json::Value) -> Result<Settings, String> {
    let raw = config
        .get("skills")
        .and_then(|s| s.get("mindfulness_spirit"))
        .filter(|v| v.is_object());

    let persona_slug = raw
        .and_then(|r| r.get("persona_slug"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or(
            "missing skills.mindfulness_spirit.persona_slug in ~/.nullclaw/config.json",
        )?
        .to_string();

    let column_slug = raw
        .and_then(|r| r.get("column_slug"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_COLUMN_SLUG)
        .to_string();

    Ok(Settings {
        persona_slug,
        column_slug,
        publish: raw
            .and_then(|r| r.get("publish"))
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
        main_image_url: raw
            .and_then(|r| r.get("main_image_url"))
            .and_then(|v| v.as_str())
            .map(str::to_string),
    })
}
