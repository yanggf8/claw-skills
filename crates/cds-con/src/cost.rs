//! Corporate bond cost — attribute 2 of the finance-engineering research.
//!
//! **`finance-cli` is the authority for this rule, not this file.** The owner's
//! ruling of 2026-08-12 retired the charter and made `cost level` the single
//! definition of attribute 2; `attribute2()` there delegates to
//! `cost_cmd::level_at`. This module exists so the daily push can state the
//! same reading, and it must stay arithmetically identical to that function —
//! including the integer truncation below, which decides the label on the
//! boundary. If the two ever disagree, `finance-cli` is right.
//!
//! **What is measured is the Baa yield itself, never a spread.** The Baa−Aaa
//! quality spread answers a different question — how much more junk costs than
//! quality, i.e. credit stratification — not whether borrowing was expensive.
//! A high-cost year can be compressing and a low-cost year widening, which is
//! why the direction measure could not separate the anchors: five of six
//! carried one label, and 1966 (50th percentile) sat in the same class as 1999
//! (12th).
//!
//! **The basis is an as-of expanding window**: only observations up to and
//! including the one being read, never the future. Early windows are thin, so
//! `n` travels with every reading rather than being hidden — thinness is a
//! fact about the reading, not a blemish to conceal.

use credit_store::{below_and_total, Observation};

/// The cut: an as-of percentile at or above this is `高`. Owner's ruling,
/// 2026-08-12 — the median, matching `cost_cmd::HIGH_PCT`.
pub const HIGH_PCT: usize = 50;

/// The series this attribute is defined over. Nothing else may be substituted:
/// a spread series here would silently answer a different question.
pub const COST_SERIES: &str = "baa";

#[derive(Debug, Clone, PartialEq)]
pub struct Level {
    pub date: String,
    pub value: f64,
    /// As-of percentile rank, truncated to a whole number.
    pub pct: usize,
    /// Observations in the as-of window, including this one.
    pub n: usize,
}

pub fn label(pct: usize) -> &'static str {
    if pct >= HIGH_PCT {
        "高"
    } else {
        "不高"
    }
}

/// The reading for one date. `None` when the series does not carry that date —
/// which is "no observation", a different statement from a zero percentile.
///
/// `rows` must be in ascending date order, as `read_credit_history` returns
/// them.
pub fn level_at(rows: &[Observation], date: &str) -> Option<Level> {
    let value = rows.iter().find(|r| r.date == date).map(|r| r.value)?;
    // As-of: everything up to and including this date, and nothing after it.
    let window: Vec<f64> = rows
        .iter()
        .filter(|r| r.date.as_str() <= date)
        .map(|r| r.value)
        .collect();
    let (below, n) = below_and_total(&window, value);
    Some(Level {
        date: date.to_string(),
        value,
        // Integer division, truncating — `cost_cmd::level_at` does exactly
        // this, and rounding here would flip the label on any window whose
        // share lands between 49.5 and 50.
        pct: below * 100 / n,
        n,
    })
}

/// The reading for the newest observation, which is what a daily push reports.
pub fn latest_level(rows: &[Observation]) -> Option<Level> {
    level_at(rows, &rows.last()?.date)
}
