//! Config load and default series.
//! Line-by-line translation of inflation-con/scripts/run.py
//! `DEFAULT_SERIES`, `VALID_STANCES`, `load_config`.

use std::path::Path;

/// FRED series → role. Insertion order matches run.py:55-63.
/// Core PCE is primary; the rest are confirmation or context.
pub const DEFAULT_SERIES: &[(&str, &str)] = &[
    ("core_pce", "PCEPILFE"),       // primary
    ("core_cpi", "CPILFESL"),       // confirmation
    ("headline_pce", "PCEPI"),      // context
    ("headline_cpi", "CPIAUCSL"),   // context
    ("breakeven_10y", "T10YIE"),    // market expectations (daily)
    ("real_yield_10y", "DFII10"),   // context (daily)
    ("nominal_10y", "DGS10"),       // context (daily)
];

/// run.py:65  VALID_STANCES = {"restrictive", "neutral", "easing", "unclear"}
pub const VALID_STANCES: &[&str] = &["restrictive", "neutral", "easing", "unclear"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Series entries in document / DEFAULT_SERIES order (Python dict
    /// insertion order). Not a BTreeMap: alphabetical iteration would
    /// reorder the warning join that fetch_all builds.
    pub series: Vec<(String, String)>,
    pub policy_stance: String,
}

fn default_series_vec() -> Vec<(String, String)> {
    DEFAULT_SERIES
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

/// Load config from `path`. Missing file → defaults (no panic).
/// Malformed / unreadable file panics — same as Python's load_config
/// running outside main's try (wart preserved).
///
/// run.py:77-88:
///   series = dict(DEFAULT_SERIES); series.update(cfg.get("series", {}))
///   stance = str(...).strip().lower(); in VALID_STANCES or "unclear"
pub fn load_config(path: &Path) -> Config {
    if !path.exists() {
        return Config {
            series: default_series_vec(),
            policy_stance: "unclear".into(),
        };
    }
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("load_config: read {}: {e}", path.display()));
    let v: serde_json::Value = serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("load_config: parse {}: {e}", path.display()));

    // Start from DEFAULT_SERIES (document order), then apply file overrides.
    // Existing keys keep DEFAULT_SERIES position; new keys append in file
    // insertion order. File order for extras needs serde_json preserve_order.
    let mut series = default_series_vec();
    if let Some(obj) = v.get("series").and_then(|s| s.as_object()) {
        for (k, val) in obj {
            let s = val
                .as_str()
                .map(|s| s.to_string())
                .unwrap_or_else(|| val.to_string());
            if let Some(slot) = series.iter_mut().find(|(key, _)| key == k) {
                slot.1 = s;
            } else {
                series.push((k.clone(), s));
            }
        }
    }

    let stance_raw = v
        .get("policy_stance")
        .and_then(|p| p.as_str())
        .unwrap_or("unclear");
    let stance = stance_raw.trim().to_lowercase();
    let policy_stance = if VALID_STANCES.contains(&stance.as_str()) {
        stance
    } else {
        "unclear".into()
    };

    Config {
        series,
        policy_stance,
    }
}
