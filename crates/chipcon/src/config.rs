//! Config load and default manual-events list.
//! Line-by-line translation of chipcon/scripts/run.py `default_events` / `load_config`.

use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Symbol entries in config-document order (Python dict insertion order).
    /// Not a BTreeMap: alphabetical iteration would reorder the fatal-path
    /// warning join that nullclaw stores in cron_runs.output.
    pub symbols: Vec<(String, String)>,
    pub position_label: String,
    pub manual_events: Vec<String>,
}

pub fn default_events() -> Vec<String> {
    vec![
        "NVDA / AVGO / AMD / MU guidance".into(),
        "TSMC monthly revenue".into(),
        "Microsoft / Amazon / Google / Meta capex guidance".into(),
        "Export-control escalation".into(),
        "SpaceX IPO / index-flow liquidity drain".into(),
    ]
}

fn default_symbols() -> Vec<(String, String)> {
    // Same insertion order as chipcon/config.json and the Python defaults.
    vec![
        ("SMH".into(), "SMH".into()),
        ("QQQ".into(), "QQQ".into()),
        ("SOXX".into(), "SOXX".into()),
    ]
}

/// Load config from `path`. Missing file → defaults (no panic).
/// Malformed / unreadable file panics — same as Python's load_config running
/// outside main's try (wart 1 preserved).
pub fn load_config(path: &Path) -> Config {
    if !path.exists() {
        return Config {
            symbols: default_symbols(),
            position_label: String::new(),
            manual_events: default_events(),
        };
    }
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("load_config: read {}: {e}", path.display()));
    let v: serde_json::Value = serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("load_config: parse {}: {e}", path.display()));

    // Document order: serde_json's Map preserves insertion order only when the
    // `preserve_order` feature is enabled (see Cargo.toml). Without it, Map is
    // BTreeMap-backed and would iterate alphabetically.
    let symbols = match v.get("symbols").and_then(|s| s.as_object()) {
        Some(obj) => obj
            .iter()
            .map(|(k, val)| {
                let s = val
                    .as_str()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| val.to_string());
                (k.clone(), s)
            })
            .collect(),
        None => default_symbols(),
    };

    let manual_events = match v.get("manual_events").and_then(|e| e.as_array()) {
        Some(arr) => arr
            .iter()
            .map(|x| {
                x.as_str()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| x.to_string())
            })
            .collect(),
        None => default_events(),
    };

    let position_label = v
        .get("position_label")
        .and_then(|p| p.as_str())
        .unwrap_or("")
        .to_string();

    Config {
        symbols,
        position_label,
        manual_events,
    }
}
