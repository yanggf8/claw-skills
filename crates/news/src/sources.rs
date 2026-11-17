//! Where the AI section's candidates come from: `NEWS_AI_SOURCES`.
//!
//! Spec `docs/specs/2026-11-17-news-ai-direct-feeds-design.md` §6. Only an
//! unset or empty variable means `google`. Any other value that is not
//! exactly `google` or `direct` — a typo like `direc` — must not silently
//! become Google: the caller alerts and builds Google so the reader still
//! gets a digest, and the marker stays `ok` because `degraded` under
//! `retry_once` would re-deliver the whole digest.

use crate::alert::{alert_failure, AlertContext};
use crate::config::{AI_SUBSTAGE_CACHE_VARIANT, AI_SUBSTAGE_CACHE_VARIANT_DIRECT};

pub const AI_SOURCES_ENV: &str = "NEWS_AI_SOURCES";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiSources {
    /// The four Google News search feeds — today's behaviour, byte-identical.
    Google,
    /// The operator's publisher feeds, `~/.nullclaw/news-ai-feeds.json`.
    Direct,
}

impl AiSources {
    pub fn as_str(self) -> &'static str {
        match self {
            AiSources::Google => "google",
            AiSources::Direct => "direct",
        }
    }

    /// The AI substage day-cache variant [R1]. Distinct per mode, so neither
    /// can replay the other's section; Google keeps today's value.
    pub fn cache_variant(self) -> &'static str {
        match self {
            AiSources::Google => AI_SUBSTAGE_CACHE_VARIANT,
            AiSources::Direct => AI_SUBSTAGE_CACHE_VARIANT_DIRECT,
        }
    }
}

/// What the variable asked for, before anything acts on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AiSourcesSetting {
    Google,
    Direct,
    /// The value seen, verbatim.
    Invalid(String),
}

/// Exact names only: no trimming, no case folding. A value that needs either
/// is a typo the operator should hear about, not one to guess at.
pub fn parse_ai_sources(raw: Option<&str>) -> AiSourcesSetting {
    match raw {
        None | Some("") | Some("google") => AiSourcesSetting::Google,
        Some("direct") => AiSourcesSetting::Direct,
        Some(other) => AiSourcesSetting::Invalid(other.to_string()),
    }
}

/// Read the variable now — never cached, so each run sees its own env.
pub fn ai_sources_setting() -> AiSourcesSetting {
    match std::env::var_os(AI_SOURCES_ENV) {
        None => parse_ai_sources(None),
        Some(v) => parse_ai_sources(Some(&v.to_string_lossy())),
    }
}

/// The mode this run builds with. An invalid value raises
/// `news_ai_sources_invalid` and falls back to Google; it does not touch the
/// run's status.
pub fn resolve_ai_sources(ctx: &AlertContext) -> AiSources {
    match ai_sources_setting() {
        AiSourcesSetting::Google => AiSources::Google,
        AiSourcesSetting::Direct => AiSources::Direct,
        AiSourcesSetting::Invalid(value) => {
            alert_failure(
                ctx,
                "news_ai_sources_invalid",
                &format!(
                    "{AI_SOURCES_ENV}={value:?} is not google|direct; built the AI section from google"
                ),
            );
            AiSources::Google
        }
    }
}
