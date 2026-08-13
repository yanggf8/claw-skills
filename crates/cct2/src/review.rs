//! Holding the morning's prediction to the day's close.
//!
//! Pure: it takes a stored prediction and a closing price and returns a
//! verdict. No clock, no network, no files — the scoring rule is the part worth
//! pinning, and it is pinned without either.

use crate::journal::Prediction;

/// How far the close must move before a directional call counts as right.
///
/// A judgement call, and stated as one. `neutral` needs a band or it can never
/// be right — a close is virtually never unchanged to the cent — and once
/// neutral has a band, `bullish` and `bearish` must use the same one or a
/// single day could satisfy two directions at once. Half a percent is the
/// figure chosen; the report prints every actual percentage beside its verdict,
/// so a reader who disagrees with the band can still see what happened.
pub const NEUTRAL_BAND_PCT: f64 = 0.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Hit,
    Miss,
    /// No direction was predicted, or a price is missing on either side. Not a
    /// miss: a reading that could not be scored is not a reading that was
    /// wrong, and counting it as one would quietly deflate every score.
    Unscored,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Reviewed {
    pub ticker: String,
    pub predicted: String,
    pub confidence: f64,
    pub reference_price: Option<f64>,
    pub close_price: Option<f64>,
    pub pct_change: Option<f64>,
    pub outcome: Outcome,
}

/// Percent move from the price the prediction was made against to the close.
///
/// `None` when either side is missing or the reference is zero — an unusable
/// divisor, the same guard `market::parse_quote` applies.
pub fn pct_change(reference: Option<f64>, close: Option<f64>) -> Option<f64> {
    let (r, c) = (reference?, close?);
    if r == 0.0 {
        return None;
    }
    Some((c - r) / r * 100.0)
}

pub fn score(sentiment: &str, pct: Option<f64>) -> Outcome {
    let Some(p) = pct else {
        return Outcome::Unscored;
    };
    match sentiment {
        "bullish" => {
            if p > NEUTRAL_BAND_PCT {
                Outcome::Hit
            } else {
                Outcome::Miss
            }
        }
        "bearish" => {
            if p < -NEUTRAL_BAND_PCT {
                Outcome::Hit
            } else {
                Outcome::Miss
            }
        }
        "neutral" => {
            if p.abs() <= NEUTRAL_BAND_PCT {
                Outcome::Hit
            } else {
                Outcome::Miss
            }
        }
        // Includes the empty string: a model that named no direction made no
        // prediction, and there is nothing to be right or wrong about.
        _ => Outcome::Unscored,
    }
}

/// Review every stored prediction against the closes now in hand.
pub fn review(predictions: &[Prediction], close_of: &dyn Fn(&str) -> Option<f64>) -> Vec<Reviewed> {
    predictions
        .iter()
        .map(|p| {
            let close = close_of(&p.ticker);
            let pct = pct_change(p.reference_price, close);
            Reviewed {
                ticker: p.ticker.clone(),
                predicted: p.sentiment.clone(),
                confidence: p.confidence,
                reference_price: p.reference_price,
                close_price: close,
                pct_change: pct,
                outcome: score(&p.sentiment, pct),
            }
        })
        .collect()
}

/// Hits and how many were scorable at all. Never a percentage: with five
/// tickers a rate reads as precision the sample cannot carry.
pub fn tally(rows: &[Reviewed]) -> (usize, usize) {
    let hits = rows.iter().filter(|r| r.outcome == Outcome::Hit).count();
    let scored = rows
        .iter()
        .filter(|r| r.outcome != Outcome::Unscored)
        .count();
    (hits, scored)
}
