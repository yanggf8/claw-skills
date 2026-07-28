//! Load dotenv-style key=value pairs into the process environment.
//!
//! Mirrors weather/scripts/run.py:62-75. Missing file is a silent no-op.
//! Values use successive character-set stripping (B11), not paired-quote
//! removal. Existing env vars are never overridden (B12).

use std::path::{Path, PathBuf};

/// Path is `explicit`, else `$CLAW_ENV`, else `~/.nullclaw/.env`.
/// Sets process env vars as a side effect; returns nothing.
pub fn load_env(explicit: Option<&Path>) {
    let path = resolve_env_path(explicit);
    let Ok(body) = std::fs::read_to_string(&path) else {
        return;
    };
    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || !line.contains('=') {
            continue;
        }
        let (key, _, val) = partition_first_eq(line);
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        // B11: successive character-set stripping, not paired-quote removal.
        // Python: val.strip().strip('"').strip("'")
        let val = val
            .trim()
            .trim_matches('"')
            .trim_matches('\'');
        // B12: set only if the key is absent from the environment.
        if std::env::var_os(key).is_none() {
            std::env::set_var(key, val);
        }
    }
}

fn resolve_env_path(explicit: Option<&Path>) -> PathBuf {
    if let Some(p) = explicit {
        return p.to_path_buf();
    }
    match std::env::var("CLAW_ENV") {
        Ok(v) if !v.is_empty() => PathBuf::from(v),
        _ => {
            let home = std::env::var("HOME").unwrap_or_default();
            PathBuf::from(home).join(".nullclaw/.env")
        }
    }
}

/// Split on the first `=` only — matches Python `line.partition("=")`.
fn partition_first_eq(s: &str) -> (&str, &str, &str) {
    match s.find('=') {
        Some(i) => (&s[..i], "=", &s[i + 1..]),
        None => (s, "", ""),
    }
}
