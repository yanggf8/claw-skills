//! Location-name resolution.
//!
//! Ports `resolve()` from traffic/scripts/run.py:38-50.

use std::fmt;

#[derive(Debug)]
pub enum ResolveError {
    Unknown(String),
}

impl fmt::Display for ResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // Reproduced from run.py:50. This reaches the user through the
            // `[WARN: traffic unavailable - {e}]` line, so the wording is part
            // of the delivered output rather than an internal detail.
            ResolveError::Unknown(name) => write!(
                f,
                "Unknown location: '{name}'. Add it to locations.json or use 'lat,lon'."
            ),
        }
    }
}

impl std::error::Error for ResolveError {}

/// Resolve a location name, or a raw `lat,lon`, to the coordinate string the
/// TomTom URL is built from.
///
/// The table is a slice of pairs rather than a map because locations.json is
/// small and read once; ordering is irrelevant and the first match wins either
/// way.
pub fn resolve(name: &str, table: &[(String, String)]) -> Result<String, ResolveError> {
    // The table is consulted first and unconditionally (run.py:40), so an entry
    // named "1,2" shadows the coordinate reading of the same text.
    if let Some((_, coords)) = table.iter().find(|(key, _)| key == name) {
        return Ok(coords.clone());
    }

    let parts: Vec<&str> = name.split(',').collect();
    if parts.len() == 2
        && parts[0].trim().parse::<f64>().is_ok()
        && parts[1].trim().parse::<f64>().is_ok()
    {
        // Python returns `name`, not the trimmed parts (run.py:47). Returning a
        // cleaned-up version here would change the request URL.
        return Ok(name.to_string());
    }

    Err(ResolveError::Unknown(name.to_string()))
}
