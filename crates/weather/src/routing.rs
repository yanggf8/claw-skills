//! HK-vs-TW location routing and argument defaults.
//!
//! Oracle: weather/scripts/run.py `is_hk_location` and
//! `locations = args.locations or ["臺北市"]`.

/// Exact closed set from run.py:18. Matching is on lowercased+trimmed input;
/// set members are already lowercase / CJK-invariant.
const HK_LOCATIONS: &[&str] = &["香港", "hong kong", "hk", "九龍", "新界", "港島"];

/// True iff `loc` is a Hong Kong alias after `to_lowercase` + `trim`.
/// Everything else is treated as Taiwan.
pub fn is_hk(loc: &str) -> bool {
    let lowered = loc.to_lowercase();
    HK_LOCATIONS.contains(&lowered.trim())
}

/// Partition into (hk, tw) preserving input order and duplicates.
/// Original strings are kept (not normalized).
pub fn split(locations: &[String]) -> (Vec<String>, Vec<String>) {
    let mut hk = Vec::new();
    let mut tw = Vec::new();
    for loc in locations {
        if is_hk(loc) {
            hk.push(loc.clone());
        } else {
            tw.push(loc.clone());
        }
    }
    (hk, tw)
}

/// Apply the Python `args.locations or ["臺北市"]` default.
/// Both `None` (caller) and an empty list take the default — empty is falsy.
pub fn with_default(locations: Vec<String>) -> Vec<String> {
    if locations.is_empty() {
        vec!["臺北市".to_string()]
    } else {
        locations
    }
}
