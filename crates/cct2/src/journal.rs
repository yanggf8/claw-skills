//! What the pre-market run predicted, kept so the close can be held to it.
//!
//! cct2 was stateless: each run fetched, asked two models, rendered, exited.
//! Nothing survived, so the end-of-day report could only ever produce another
//! forecast — there was no record of the morning's to check it against.
//!
//! One file per **ET business date**, holding each ticker's direction and the
//! price the prediction was made against. The reference price is stored rather
//! than re-fetched at review time: the close must be compared to the number the
//! model actually saw, and re-deriving "yesterday's close" hours later is a
//! different quantity whenever a split, a dividend or a data revision lands in
//! between.
//!
//! Written atomically. A half-written file parses as absent, which would
//! silently turn a real prediction into "no review available" — the one failure
//! this record exists to prevent.

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::merge::Row;

#[derive(Debug, Clone, PartialEq)]
pub struct Prediction {
    pub ticker: String,
    pub sentiment: String,
    pub confidence: f64,
    /// The price the model was shown, in market currency. `None` when the quote
    /// was unavailable — the direction is still worth keeping, the magnitude
    /// simply cannot be scored.
    pub reference_price: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Journal {
    pub business_date: String,
    /// Market-time stamp of the run that wrote this, e.g. `08:30 EDT`.
    pub made_at: String,
    pub predictions: Vec<Prediction>,
}

pub fn journal_dir(home: &Path) -> PathBuf {
    home.join(".nullclaw/skills/cct2/journal")
}

pub fn journal_path(home: &Path, business_date: &str) -> PathBuf {
    journal_dir(home).join(format!("{business_date}.json"))
}

/// Pair each row with the price its ticker was quoted at.
pub fn predictions_from(rows: &[Row], price_of: &dyn Fn(&str) -> Option<f64>) -> Vec<Prediction> {
    rows.iter()
        .map(|r| Prediction {
            ticker: r.ticker.clone(),
            sentiment: r.sentiment.clone(),
            confidence: r.confidence,
            reference_price: price_of(&r.ticker),
        })
        .collect()
}

pub fn to_json(j: &Journal) -> serde_json::Value {
    serde_json::json!({
        "business_date": j.business_date,
        "made_at": j.made_at,
        "predictions": j.predictions.iter().map(|p| serde_json::json!({
            "ticker": p.ticker,
            "sentiment": p.sentiment,
            "confidence": p.confidence,
            "reference_price": p.reference_price,
        })).collect::<Vec<_>>(),
    })
}

/// Parse a stored journal. `None` for anything unreadable — a corrupt record is
/// the same as no record, and must never be reported as a prediction.
pub fn from_json(v: &serde_json::Value) -> Option<Journal> {
    let business_date = v.get("business_date")?.as_str()?.to_string();
    if business_date.is_empty() {
        return None;
    }
    let predictions = v
        .get("predictions")?
        .as_array()?
        .iter()
        .filter_map(|p| {
            let ticker = p.get("ticker")?.as_str()?.to_string();
            if ticker.is_empty() {
                return None;
            }
            Some(Prediction {
                ticker,
                sentiment: p
                    .get("sentiment")
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .to_string(),
                confidence: p
                    .get("confidence")
                    .and_then(|c| c.as_f64())
                    .unwrap_or(0.0),
                // Absent and JSON null both mean "no price was recorded"; a
                // non-numeric value is the same answer, not a reason to throw
                // the whole prediction away.
                reference_price: p.get("reference_price").and_then(|x| x.as_f64()),
            })
        })
        .collect();
    Some(Journal {
        business_date,
        made_at: v
            .get("made_at")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string(),
        predictions,
    })
}

/// Write the journal for its business date, creating the directory if needed.
///
/// Temp-then-rename: a reader that lands mid-write must see either the old file
/// or the new one, never a truncated one that parses as absent.
pub fn save(home: &Path, j: &Journal) -> std::io::Result<PathBuf> {
    let dir = journal_dir(home);
    std::fs::create_dir_all(&dir)?;
    let final_path = journal_path(home, &j.business_date);
    let tmp = dir.join(format!(".{}.json.tmp", j.business_date));
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(to_json(j).to_string().as_bytes())?;
        f.write_all(b"\n")?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, &final_path)?;
    Ok(final_path)
}

/// The journal for a business date, or `None` if there is not a usable one.
pub fn load(home: &Path, business_date: &str) -> Option<Journal> {
    let text = std::fs::read_to_string(journal_path(home, business_date)).ok()?;
    from_json(&serde_json::from_str::<serde_json::Value>(&text).ok()?)
}

// ── which models actually answered ───────────────────────────────────────────
//
// A model can fail without the run failing. When MiniMax-M2.7 spent its whole
// token budget on a thinking block it returned no text, the report shipped on
// the backup alone, and the run was legitimately `[skill-status:ok]` — the only
// trace was a `WARN` on stderr, which nothing keeps: `cron_runs.output` holds
// the ~74 bytes of marker lines and no more. So the frequency of a silent
// half-outage was unanswerable after the fact, and had to be estimated by
// noticing that a consensus confidence can only end in `.xx5` if two models
// were averaged. That is a hint, not a record. This is the record.
//
// Appended, one line per run, rather than folded into the day file: the day
// file is the morning's prediction record that the close is scored against, and
// the close would have to read-modify-write it to add its own line. A crash in
// that window trades a real prediction for "no review available" — the single
// failure this module exists to prevent. An append cannot lose the morning, and
// it keeps the retry's second attempt instead of overwriting the first.

/// One model's showing on one run.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelUse {
    pub model: String,
    /// The model returned a reply this run's merge could read.
    pub answered: bool,
    /// How many tickers it actually gave an opinion on. A model can answer and
    /// still omit a ticker, which is why `answered` alone does not settle
    /// whether the report was a real two-model comparison.
    pub tickers: usize,
}

/// What the two models did on one run.
#[derive(Debug, Clone, PartialEq)]
pub struct RunModels {
    pub business_date: String,
    /// `pre-market` or `eod`.
    pub mode: String,
    /// Market-time stamp of the run, e.g. `08:30 EDT`.
    pub made_at: String,
    pub primary: ModelUse,
    pub backup: ModelUse,
    /// Tickers the run set out to cover, so a shortfall is visible rather than
    /// implied by a smaller per-model count.
    pub requested: usize,
}

pub fn models_path(home: &Path) -> PathBuf {
    journal_dir(home).join("models.jsonl")
}

pub fn models_to_json(r: &RunModels) -> serde_json::Value {
    let one = |m: &ModelUse| {
        serde_json::json!({
            "model": m.model,
            "answered": m.answered,
            "tickers": m.tickers,
        })
    };
    serde_json::json!({
        "business_date": r.business_date,
        "mode": r.mode,
        "made_at": r.made_at,
        "requested": r.requested,
        "primary": one(&r.primary),
        "backup": one(&r.backup),
    })
}

/// Append one run's record, creating the directory and file if needed.
///
/// One line of JSON per run: `grep`-able without a parser, and an interrupted
/// append costs the newest line rather than the file.
pub fn append_models(home: &Path, r: &RunModels) -> std::io::Result<PathBuf> {
    let dir = journal_dir(home);
    std::fs::create_dir_all(&dir)?;
    let path = models_path(home);
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    f.write_all(models_to_json(r).to_string().as_bytes())?;
    f.write_all(b"\n")?;
    f.sync_all()?;
    Ok(path)
}
