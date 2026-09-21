//! Combining the two models' answers for one ticker.

/// How the two models related to each other on this ticker.
///
/// This replaces a pair of overlapping booleans (`consensus` and `diverged`)
/// that could not express the three cases without contradiction. In the Python
/// a ticker answered by only one model was written `consensus: not both_present`
/// — that is, **true** — so the report filed it under 🎯 共識訊號 and the
/// 📊 單一參考 section, whose filter was `not consensus and not diverged`, could
/// never match anything. A reader was told two models agreed when one had
/// spoken. Three variants of one enum cannot overlap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Agreement {
    /// Both answered, same direction.
    Consensus,
    /// Both answered, different directions.
    Diverged,
    /// Exactly one answered.
    Solo,
}

/// One model's read on one ticker.
#[derive(Debug, Clone, PartialEq)]
pub struct Opinion {
    pub sentiment: String,
    pub confidence: f64,
    pub reason: String,
}

impl Opinion {
    /// An opinion counts as present only when it names a direction. A row with
    /// a confidence but no sentiment says nothing.
    pub fn is_present(&self) -> bool {
        !self.sentiment.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub ticker: String,
    pub agreement: Agreement,
    /// The direction to report: the agreed one, or the only one there is. For a
    /// divergence this is the primary's, and the report shows both sides.
    pub sentiment: String,
    /// Averaged on consensus; otherwise the reporting model's own.
    pub confidence: f64,
    pub reason: String,
    pub primary: Opinion,
    pub backup: Opinion,
}

fn opinion(map: Option<&serde_json::Value>, ticker: &str) -> Opinion {
    let o = map.and_then(|m| m.get(ticker));
    Opinion {
        sentiment: o
            .and_then(|v| v.get("sentiment"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_lowercase(),
        // `float(p.get("confidence") or 0)` — absent, null and 0 all become 0.
        confidence: o
            .and_then(|v| v.get("confidence"))
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
        reason: o
            .and_then(|v| v.get("reason"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
    }
}

/// Build one row per ticker that at least one model answered.
///
/// Tickers neither model spoke about are dropped rather than reported as
/// unknown — that is the Python's behaviour and it is right: a silent model is
/// not a neutral reading.
pub fn merge(
    tickers: &[String],
    primary: Option<&serde_json::Value>,
    backup: Option<&serde_json::Value>,
) -> Vec<Row> {
    let mut rows: Vec<Row> = Vec::new();

    for t in tickers {
        let p = opinion(primary, t);
        let b = opinion(backup, t);

        let (agreement, sentiment, confidence) = match (p.is_present(), b.is_present()) {
            (false, false) => continue,
            (true, true) if p.sentiment == b.sentiment => (
                Agreement::Consensus,
                p.sentiment.clone(),
                (p.confidence + b.confidence) / 2.0,
            ),
            (true, true) => (Agreement::Diverged, p.sentiment.clone(), p.confidence),
            (true, false) => (Agreement::Solo, p.sentiment.clone(), p.confidence),
            (false, true) => (Agreement::Solo, b.sentiment.clone(), b.confidence),
        };

        let reason = if p.reason.is_empty() {
            b.reason.clone()
        } else {
            p.reason.clone()
        };

        rows.push(Row {
            ticker: t.clone(),
            agreement,
            sentiment,
            confidence,
            reason,
            primary: p,
            backup: b,
        });
    }

    // Divergences first — they are the ones asking for a decision — then by
    // confidence, highest first.
    rows.sort_by(|a, b| {
        let rank = |r: &Row| if r.agreement == Agreement::Diverged { 0 } else { 1 };
        rank(a)
            .cmp(&rank(b))
            .then(b.confidence.partial_cmp(&a.confidence).unwrap_or(std::cmp::Ordering::Equal))
    });
    rows
}
