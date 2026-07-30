//! Pure classification layer — no IO, no network, no clock.
//! Line-by-line translation of chipcon/scripts/run.py helpers.

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub day: String,
    pub close: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok,
    Yellow,
    Orange,
    Red,
    ProfitProtect,
    InsufficientHistory,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Details {
    pub day: String,
    pub current: f64,
    pub ma20: Option<f64>,
    pub ma50: Option<f64>,
    pub rising20: Option<bool>,
    pub smh5: Option<f64>,
    pub qqq5: Option<f64>,
    pub soxx5: Option<f64>,
    pub rel_qqq5: Option<f64>,
    pub rel_soxx5: Option<f64>,
    pub down_days: usize,
    pub distance20: Option<f64>,
    pub distance50: Option<f64>,
    pub rows: usize,
    pub reasons: Vec<String>,
}

pub fn pct(a: f64, b: f64) -> f64 {
    (a / b - 1.0) * 100.0
}

pub fn ma(rows: &[Row], n: usize) -> Option<f64> {
    if rows.len() < n {
        return None;
    }
    let sum: f64 = rows[rows.len() - n..].iter().map(|r| r.close).sum();
    Some(sum / n as f64)
}

pub fn ma_rising(rows: &[Row], n: usize, lookback: usize) -> Option<bool> {
    if rows.len() < n + lookback {
        return None;
    }
    let current = ma(rows, n);
    let past = ma(&rows[..rows.len() - lookback], n);
    match (current, past) {
        (Some(c), Some(p)) => Some(c > p),
        _ => None,
    }
}

pub fn return_n(rows: &[Row], n: usize) -> Option<f64> {
    if rows.len() <= n {
        return None;
    }
    Some(pct(rows[rows.len() - 1].close, rows[rows.len() - 1 - n].close))
}

pub fn consecutive_down(rows: &[Row]) -> usize {
    let mut count = 0;
    // Python: for idx in range(len(rows) - 1, 0, -1)
    let mut idx = rows.len();
    while idx > 1 {
        idx -= 1;
        if rows[idx].close < rows[idx - 1].close {
            count += 1;
        } else {
            break;
        }
    }
    count
}

pub fn classify(smh: &[Row], qqq: &[Row], soxx: &[Row]) -> (Status, Details) {
    if smh.len() < 20 {
        return (
            Status::InsufficientHistory,
            Details {
                day: String::new(),
                current: 0.0,
                ma20: None,
                ma50: None,
                rising20: None,
                smh5: None,
                qqq5: None,
                soxx5: None,
                rel_qqq5: None,
                rel_soxx5: None,
                down_days: 0,
                distance20: None,
                distance50: None,
                rows: smh.len(),
                reasons: vec![],
            },
        );
    }

    let current_day = smh[smh.len() - 1].day.clone();
    let current = smh[smh.len() - 1].close;
    let ma20 = ma(smh, 20);
    let ma50 = ma(smh, 50);
    let rising20 = ma_rising(smh, 20, 5);
    let smh5 = return_n(smh, 5);
    let qqq5 = return_n(qqq, 5);
    let soxx5 = return_n(soxx, 5);
    let rel_qqq5 = match (smh5, qqq5) {
        (Some(s), Some(q)) => Some(s - q),
        _ => None,
    };
    let rel_soxx5 = match (smh5, soxx5) {
        (Some(s), Some(x)) => Some(s - x),
        _ => None,
    };
    let down_days = consecutive_down(smh);
    let distance20 = ma20.map(|m| pct(current, m));
    let distance50 = ma50.map(|m| pct(current, m));

    let mut status = Status::Ok;
    let mut reasons: Vec<String> = vec![];

    // RED — three independent ifs (accumulate)
    if ma50.is_some() && current < ma50.unwrap() {
        status = Status::Red;
        reasons.push("SMH below 50DMA".into());
    }
    if ma20.is_some() && ma50.is_some() && ma20.unwrap() < ma50.unwrap() {
        status = Status::Red;
        reasons.push("20DMA below 50DMA".into());
    }
    if let Some(r) = rel_qqq5 {
        if r <= -6.0 {
            status = Status::Red;
            reasons.push("SMH underperformed QQQ by 6%+ over 5d".into());
        }
    }

    // ORANGE — three independent ifs (accumulate)
    if status == Status::Ok {
        if ma20.is_some() && current < ma20.unwrap() && rising20 == Some(false) {
            status = Status::Orange;
            reasons.push("SMH below 20DMA and 20DMA falling".into());
        }
        if let Some(r) = rel_qqq5 {
            if r <= -4.0 {
                status = Status::Orange;
                reasons.push("SMH underperformed QQQ by 4%+ over 5d".into());
            }
        }
        if down_days >= 3 && ma20.is_some() && current < ma20.unwrap() {
            status = Status::Orange;
            reasons.push("3+ down days and below 20DMA".into());
        }
    }

    // YELLOW — if / elif / elif (at most one reason)
    if status == Status::Ok {
        if ma20.is_some() && current < ma20.unwrap() {
            status = Status::Yellow;
            reasons.push("SMH below 20DMA".into());
        } else if rising20 == Some(false) {
            status = Status::Yellow;
            reasons.push("20DMA no longer rising".into());
        } else if let Some(r) = rel_qqq5 {
            if r <= -2.0 {
                status = Status::Yellow;
                reasons.push("SMH underperformed QQQ by 2%+ over 5d".into());
            }
        }
    }

    // PROFIT_PROTECT
    if status == Status::Ok {
        if let Some(d20) = distance20 {
            if d20 >= 8.0 && down_days >= 1 {
                status = Status::ProfitProtect;
                reasons.push("SMH still extended above 20DMA after a down day".into());
            }
        }
    }

    let details = Details {
        day: current_day,
        current,
        ma20,
        ma50,
        rising20,
        smh5,
        qqq5,
        soxx5,
        rel_qqq5,
        rel_soxx5,
        down_days,
        distance20,
        distance50,
        rows: smh.len(),
        reasons,
    };
    (status, details)
}
