//! Oil-trend analysis: moving averages, extremes, and four-state classification.
//! Line-for-line translation of the pure functions in oilcon/scripts/run.py.

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub day: String,
    pub close: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InsufficientRows;

#[derive(Debug, Clone, PartialEq)]
pub struct Extremes {
    pub current_day: String,
    pub current_close: f64,
    pub today_change_pct: f64,
    pub high_day: String,
    pub high_close: f64,
    pub days_since_high: usize,
    pub distance_from_high_pct: f64,
    pub low_day: String,
    pub low_close: f64,
    pub days_since_low: usize,
    pub distance_off_low_pct: f64,
}

pub fn moving_average(rows: &[Row], n: usize) -> Result<f64, InsufficientRows> {
    if rows.len() < n {
        return Err(InsufficientRows);
    }
    let sum: f64 = rows[rows.len() - n..].iter().map(|r| r.close).sum();
    Ok(sum / n as f64)
}

pub fn ma_rising(rows: &[Row], n: usize, lookback: usize) -> bool {
    if rows.len() < n + lookback {
        return false;
    }
    let ma_today = match moving_average(rows, n) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let ma_then = match moving_average(&rows[..rows.len() - lookback], n) {
        Ok(v) => v,
        Err(_) => return false,
    };
    ma_today > ma_then
}

pub fn pct_below_60d_high(rows: &[Row]) -> f64 {
    if rows.is_empty() {
        panic!("need at least 1 row");
    }
    let window = if rows.len() >= 60 {
        &rows[rows.len() - 60..]
    } else {
        rows
    };
    let high = window
        .iter()
        .map(|r| r.close)
        .fold(f64::NEG_INFINITY, f64::max);
    let current = rows.last().unwrap().close;
    (high - current) / high * 100.0
}

pub fn compute_change_pct(rows: &[Row]) -> f64 {
    assert!(rows.len() >= 2, "need at least 2 rows");
    let prev = rows[rows.len() - 2].close;
    let current = rows[rows.len() - 1].close;
    (current - prev) / prev * 100.0
}

pub fn compute_extremes(rows: &[Row]) -> Extremes {
    assert!(rows.len() >= 2, "need at least 2 rows");

    let current_day = rows.last().unwrap().day.clone();
    let current_close = rows.last().unwrap().close;

    // Python's `max(enumerate(rows), key=...)` returns the FIRST maximal element.
    // Rust's `Iterator::max_by` returns the LAST one, so it cannot be used here:
    // on a tied high it would report a different `high_day` and a different
    // `days_since_high`, both of which are rendered into the message. Verified
    // empirically, not read from the docs. `reduce` with a strict `>` keeps the
    // first, matching Python.
    let (high_index, high_row) = rows
        .iter()
        .enumerate()
        .reduce(|best, cur| if cur.1.close > best.1.close { cur } else { best })
        .unwrap();
    // `min_by` DOES return the first minimum, so it already matches Python. Left
    // as-is deliberately; do not "symmetrise" this with the line above.
    let (low_index, low_row) = rows
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| a.close.partial_cmp(&b.close).unwrap())
        .unwrap();

    Extremes {
        current_day,
        current_close,
        today_change_pct: compute_change_pct(rows),
        high_day: high_row.day.clone(),
        high_close: high_row.close,
        days_since_high: rows.len() - 1 - high_index,
        distance_from_high_pct: (current_close - high_row.close) / high_row.close * 100.0,
        low_day: low_row.day.clone(),
        low_close: low_row.close,
        days_since_low: rows.len() - 1 - low_index,
        distance_off_low_pct: (current_close - low_row.close) / low_row.close * 100.0,
    }
}

pub fn classify_oil_trend(rows: &[Row]) -> &'static str {
    if rows.len() < 70 {
        return "insufficient-history";
    }
    let ma50 = match moving_average(rows, 50) {
        Ok(v) => v,
        Err(_) => return "insufficient-history",
    };
    let current = rows.last().unwrap().close;
    let ma_rising_flag = ma_rising(rows, 50, 20);
    let pct_below = pct_below_60d_high(rows);

    if current > ma50 {
        if ma_rising_flag {
            if pct_below <= 10.0 {
                "uptrend"
            } else {
                "weakening-uptrend"
            }
        } else {
            "rollover"
        }
    } else if ma_rising_flag {
        "rollover"
    } else {
        "no-uptrend"
    }
}
