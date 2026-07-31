//! One quote, normalised across sources.

#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    Twse,
    Yahoo,
}

/// A single instrument's current state.
///
/// `price` is kept as the source's own string and `price_num` as its parsed
/// value, because the two answer different questions. TWSE sends "2425.0000"
/// for a stock and "-" when nothing has traded; the string is what the reader
/// should see (the trailing zeros are the exchange's own precision, and "-" is
/// information), while the number is only needed to compute the change. Parsing
/// and re-formatting the display value would silently restate the exchange's
/// figures.
#[derive(Debug, Clone, PartialEq)]
pub struct Quote {
    pub name: String,
    pub price: String,
    pub price_num: Option<f64>,
    pub prev: Option<f64>,
    pub high: Option<String>,
    pub low: Option<String>,
    /// Already rendered for display, in the exchange's own timezone.
    pub stamp: Option<String>,
    pub source: Source,
}
