//! Weather source adapters. JSON stops at this boundary — callers get Rust types.

pub mod cwa;
pub mod hko;
pub mod open_meteo;

/// Advice-summary record that accumulates into `weather_data` and feeds clothing advice.
/// `None` from a formatter means "not counted" (B7: HKO / Open-Meteo WARN paths).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub location: String,
    pub wx: String,
    pub min_t: String,
    pub max_t: String,
    pub pop: String,
}
