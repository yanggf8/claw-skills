//! Yahoo Finance quotes and headlines.

const UA: &str = "Mozilla/5.0 (compatible; nullclaw-cct2/1.0)";
const TIMEOUT_S: u64 = 15;

pub fn quote_base() -> String {
    std::env::var("CCT2_QUOTE_BASE").unwrap_or_else(|_| "https://query1.finance.yahoo.com".into())
}

pub fn search_base() -> String {
    std::env::var("CCT2_SEARCH_BASE").unwrap_or_else(|_| "https://query2.finance.yahoo.com".into())
}

#[derive(Debug, Clone, PartialEq)]
pub struct Quote {
    pub price: f64,
    pub prev_close: f64,
    pub pct_change: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TickerData {
    pub ticker: String,
    pub quote: Option<Quote>,
    pub headlines: Vec<String>,
}

fn get_json(url: &str) -> Option<serde_json::Value> {
    let resp = ureq::get(url)
        .set("User-Agent", UA)
        .set("Accept", "application/json")
        .timeout(std::time::Duration::from_secs(TIMEOUT_S))
        .call()
        .ok()?;
    if resp.status() != 200 {
        return None;
    }
    serde_json::from_str(&resp.into_string().ok()?).ok()
}

/// Round to two decimals, matching Python's `round(x, 2)`.
fn r2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

pub fn parse_quote(payload: &serde_json::Value) -> Option<Quote> {
    let result = payload.get("chart")?.get("result")?.as_array()?.first()?;
    let meta = result.get("meta")?;

    // The daily closes, with nulls dropped — Yahoo pads the series for days the
    // market did not trade.
    let closes: Vec<f64> = result
        .get("indicators")
        .and_then(|i| i.get("quote"))
        .and_then(|q| q.as_array())
        .and_then(|a| a.first())
        .and_then(|q| q.get("close"))
        .and_then(|c| c.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_f64()).collect())
        .unwrap_or_default();

    let (current, prev_close) = if closes.len() < 2 {
        let prev = meta
            .get("previousClose")
            .and_then(|v| v.as_f64())
            .or_else(|| meta.get("chartPreviousClose").and_then(|v| v.as_f64()))?;
        let cur = meta
            .get("regularMarketPrice")
            .and_then(|v| v.as_f64())
            .or_else(|| closes.last().copied())?;
        (cur, prev)
    } else {
        (closes[closes.len() - 1], closes[closes.len() - 2])
    };

    // `if not current or not prev_close` in the Python — a zero is falsy there,
    // and a zero previous close is not a usable divisor here either.
    if current == 0.0 || prev_close == 0.0 {
        return None;
    }

    Some(Quote {
        price: r2(current),
        prev_close: r2(prev_close),
        pct_change: r2((current - prev_close) / prev_close * 100.0),
    })
}

pub fn parse_headlines(payload: &serde_json::Value, max_items: usize) -> Vec<String> {
    payload
        .get("news")
        .and_then(|n| n.as_array())
        .map(|a| {
            a.iter()
                .take(max_items)
                .filter_map(|it| it.get("title").and_then(|t| t.as_str()))
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

pub fn fetch_ticker(ticker: &str) -> TickerData {
    let quote = get_json(&format!(
        "{}/v8/finance/chart/{ticker}?interval=1d&range=2d",
        quote_base()
    ))
    .as_ref()
    .and_then(parse_quote);

    let headlines = get_json(&format!(
        "{}/v1/finance/search?q={ticker}&newsCount=3&quotesCount=0",
        search_base()
    ))
    .as_ref()
    .map(|p| parse_headlines(p, 3))
    .unwrap_or_default();

    TickerData {
        ticker: ticker.to_string(),
        quote,
        headlines,
    }
}

/// The per-ticker block handed to the model.
pub fn summarise(data: &[TickerData]) -> String {
    let mut lines: Vec<String> = Vec::new();
    for d in data {
        match &d.quote {
            Some(q) => {
                let sign = if q.pct_change >= 0.0 { "+" } else { "" };
                lines.push(format!(
                    "{}: ${} ({sign}{}% vs prev close ${})",
                    d.ticker, q.price, q.pct_change, q.prev_close
                ));
            }
            None => lines.push(format!("{}: price unavailable", d.ticker)),
        }
        for h in &d.headlines {
            lines.push(format!("  - {h}"));
        }
        lines.push(String::new());
    }
    lines.join("\n")
}
