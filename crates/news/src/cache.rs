//! Day-scoped cache of LLM section output, plus its sweeper.
//!
//! Keyed by (date, prompt variant, item range). The variant is what makes a
//! prompt change safe: bump it and yesterday's answers are simply never looked
//! up, rather than being served against a prompt that no longer matches them.

use crate::config::{cache_dir, CACHE_TTL_DAYS};
use crate::trace::log_trace;
use serde_json::json;
use std::path::PathBuf;

/// `"YYYY/MM/DD (Mon)"` → `"YYYY-MM-DD"`, safe as a directory name.
fn safe_date(date_str: &str) -> String {
    date_str
        .split_whitespace()
        .next()
        .unwrap_or("")
        .replace('/', "-")
}

fn cache_path(date_str: &str, variant: &str, start: usize, end: usize) -> PathBuf {
    let dir = cache_dir().join(safe_date(date_str));
    let _ = std::fs::create_dir_all(&dir);
    dir.join(format!("{variant}-{start:03}-{end:03}.txt"))
}

pub fn get(date_str: &str, variant: &str, start: usize, end: usize) -> Option<String> {
    let path = cache_path(date_str, variant, start, end);
    match std::fs::read_to_string(&path) {
        Ok(data) => {
            log_trace(
                "news_cache_hit",
                json!({"variant": variant, "start": start, "end": end}),
            );
            Some(data)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            log_trace(
                "news_cache_read_error",
                json!({"variant": variant, "error": e.to_string()}),
            );
            None
        }
    }
}

pub fn put(date_str: &str, variant: &str, start: usize, end: usize, body: &str) {
    let path = cache_path(date_str, variant, start, end);
    match std::fs::write(&path, body) {
        Ok(()) => log_trace(
            "news_cache_write",
            json!({"variant": variant, "start": start, "end": end, "bytes": body.len()}),
        ),
        Err(e) => log_trace(
            "news_cache_write_error",
            json!({"variant": variant, "error": e.to_string()}),
        ),
    }
}

/// Drop day directories older than the TTL, and prune the separate URL-decode
/// cache. Best effort throughout — a cache that cannot be swept is not a
/// reason to fail a digest.
pub fn sweep() {
    crate::quality::sweep_decode_cache(CACHE_TTL_DAYS);

    let root = cache_dir();
    let cutoff = match std::time::SystemTime::now()
        .checked_sub(std::time::Duration::from_secs(CACHE_TTL_DAYS * 86400))
    {
        Some(t) => t,
        None => return,
    };
    let entries = match std::fs::read_dir(&root) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let stale = entry
            .metadata()
            .ok()
            .filter(|m| m.is_dir())
            .and_then(|m| m.modified().ok())
            .is_some_and(|m| m < cutoff);
        if stale {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::safe_date;

    #[test]
    fn the_weekday_suffix_never_reaches_the_path() {
        assert_eq!(safe_date("2026/08/01 (Sat)"), "2026-08-01");
    }

    #[test]
    fn a_date_without_a_suffix_still_works() {
        assert_eq!(safe_date("2026/08/01"), "2026-08-01");
    }
}
