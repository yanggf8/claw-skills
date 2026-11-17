//! Day-scoped cache of LLM section output, plus its sweeper.
//!
//! Keyed by (date, prompt variant, item range). The variant is what makes a
//! prompt change safe: bump it and yesterday's answers are simply never looked
//! up, rather than being served against a prompt that no longer matches them.

use crate::config::{cache_dir, CACHE_TTL_DAYS};
use crate::trace::log_trace;
use serde_json::json;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

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

/// Cache a rendered section, with `links` (Google link → publisher URL) for
/// the picks it rendered with a publisher URL.
pub fn put(
    date_str: &str,
    variant: &str,
    start: usize,
    end: usize,
    body: &str,
    links: &BTreeMap<String, String>,
) {
    let path = cache_path(date_str, variant, start, end);
    if !write_links_sidecar(&path, links) {
        log_trace(
            "news_cache_write_skipped",
            json!({"variant": variant, "start": start, "end": end, "reason": "links_sidecar"}),
        );
        return;
    }
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

// ── publisher-link sidecars ──────────────────────────────────────────────────
//
// A cached section is rendered text whose links are publisher URLs, while the
// protected-name check joins digest lines to source items by their Google
// link. On a cache hit the precheck never runs, so the run that replays the
// section has no record of which publisher URL stands for which Google link.
// Each cache entry therefore carries its map in a sidecar beside it.
//
// A sidecar rather than a change to the entry itself: entries are plain text
// read back verbatim, and an entry written before sidecars existed must keep
// replaying exactly as it did. Its absence means "no mapping", which is right
// for such an entry — it was rendered with Google links.

const LINKS_SUFFIX: &str = ".links.json";

fn sidecar_path(cache_file: &Path) -> PathBuf {
    let mut name = cache_file.file_name().unwrap_or_default().to_os_string();
    name.push(LINKS_SUFFIX);
    cache_file.with_file_name(name)
}

/// Write `links` beside `cache_file`, and say whether it landed.
///
/// Callers write the entry only after this returns `true`, so an entry never
/// exists without its map — a replay would otherwise blind the protected-name
/// check for the rest of the day. A map without an entry is harmless, and a
/// skipped entry costs one recomputation on the next run.
#[must_use]
pub fn write_links_sidecar(cache_file: &Path, links: &BTreeMap<String, String>) -> bool {
    let written = serde_json::to_string(links)
        .map_err(|e| e.to_string())
        .and_then(|body| std::fs::write(sidecar_path(cache_file), body).map_err(|e| e.to_string()));
    match written {
        Ok(()) => true,
        Err(error) => {
            log_trace("news_cache_links_write_error", json!({"error": error}));
            false
        }
    }
}

/// Every Google link → publisher URL mapping recorded for `date_str`'s cache
/// entries, across all sections. An unreadable sidecar is skipped: this feeds
/// an observation, never delivery.
pub fn day_publisher_links(date_str: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let Ok(entries) = std::fs::read_dir(cache_dir().join(safe_date(date_str))) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.to_string_lossy().ends_with(LINKS_SUFFIX) {
            continue;
        }
        let parsed = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<HashMap<String, String>>(&s).ok());
        if let Some(map) = parsed {
            out.extend(map);
        }
    }
    out
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
