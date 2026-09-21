//! Pure classification layer — no IO, no network, no clock.
//! Line-by-line translation of inflation-con/scripts/run.py helpers.

#[derive(Debug, Clone, PartialEq)]
pub struct Obs {
    pub day: String,
    pub value: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Series {
    pub core_pce: Vec<Obs>,
    pub core_cpi: Vec<Obs>,
    pub breakeven_10y: Vec<Obs>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok,
    Watch,
    Yellow,
    Red,
    InsufficientData,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Details {
    pub core_pce_day: String,
    pub pce3: Option<f64>,
    pub pce6: Option<f64>,
    pub cpi3: Option<f64>,
    pub cpi6: Option<f64>,
    pub breakeven: Option<f64>,
    pub breakeven_day: Option<String>,
    pub breakeven_rising: Option<bool>,
    pub policy_stance: String,
    pub core_pce_obs: usize,
    pub reasons: Vec<String>,
}

/// Compound-annualize the change over the last n monthly observations.
///
/// `((now / n-months-ago) ** (12/n) - 1) * 100`. Returns `None` if too short
/// or the base is not positive. Needs `len > n` (strict).
pub fn annualized(rows: &[Obs], n: usize) -> Option<f64> {
    if rows.len() <= n {
        return None;
    }
    let now = rows[rows.len() - 1].value;
    let then = rows[rows.len() - 1 - n].value;
    if then <= 0.0 {
        return None;
    }
    Some(((now / then).powf(12.0 / n as f64) - 1.0) * 100.0)
}

pub fn latest(rows: &[Obs]) -> Option<&Obs> {
    rows.last()
}

/// Is the latest value above the value `lookback` observations ago?
/// Needs `len > lookback` (strict).
pub fn rising_over(rows: &[Obs], lookback: usize) -> Option<bool> {
    if rows.len() <= lookback {
        return None;
    }
    Some(rows[rows.len() - 1].value > rows[rows.len() - 1 - lookback].value)
}

fn ge(v: Option<f64>, t: f64) -> bool {
    matches!(v, Some(x) if x >= t)
}

/// Classify inflation confirmation from core PCE / core CPI / breakeven.
///
/// The ladder is `if / elif / elif / else` — mutually exclusive. A RED run
/// carries no YELLOW reason. Do not port the unreachable `not core_cpi_hot`
/// branch inside YELLOW (dead code in Python).
pub fn classify(series: &Series, policy_stance: &str) -> (Status, Details) {
    let core_pce = &series.core_pce;
    let core_cpi = &series.core_cpi;
    let breakeven = &series.breakeven_10y;

    // Guard: need enough monthly core-PCE history, and a latest core-PCE/CPI.
    // Python: if len(core_pce) < 7 or not core_pce or not core_cpi
    if core_pce.len() < 7 || core_pce.is_empty() || core_cpi.is_empty() {
        return (
            Status::InsufficientData,
            Details {
                core_pce_day: String::new(),
                pce3: None,
                pce6: None,
                cpi3: None,
                cpi6: None,
                breakeven: None,
                breakeven_day: None,
                breakeven_rising: None,
                policy_stance: String::new(),
                core_pce_obs: core_pce.len(),
                reasons: vec![
                    "fewer than 7 monthly core-PCE observations or missing latest core-PCE/CPI"
                        .into(),
                ],
            },
        );
    }

    let pce3 = annualized(core_pce, 3);
    let pce6 = annualized(core_pce, 6);
    let cpi3 = annualized(core_cpi, 3);
    let cpi6 = annualized(core_cpi, 6);
    let be_last = latest(breakeven);
    let be_rising = rising_over(breakeven, 63); // ~3 months of daily obs

    // Falling trend on core PCE: 3-mo below 6-mo pace (disinflation underway).
    let pce_falling = match (pce3, pce6) {
        (Some(a), Some(b)) => a < b,
        _ => false,
    };

    let core_cpi_hot_3_or_6 = ge(cpi3, 3.0) || ge(cpi6, 3.0);
    let breakeven_ge = be_last.is_some_and(|b| b.value >= 2.5);
    let context_not_easing =
        (breakeven_ge || be_rising == Some(true)) && policy_stance != "easing";

    let mut reasons: Vec<String> = Vec::new();
    let status;

    // RED: inflation-up regime confirmed.
    if ge(pce3, 3.5) && ge(pce6, 3.5) && core_cpi_hot_3_or_6 && context_not_easing {
        status = Status::Red;
        reasons.push("core PCE 3-mo & 6-mo annualized both >= 3.5%".into());
        reasons.push("core CPI confirms (>= 3.0% on 3-mo or 6-mo)".into());
        if breakeven_ge {
            let v = be_last.unwrap().value;
            reasons.push(format!("10Y breakeven {v:.2} >= 2.5%"));
        } else if be_rising == Some(true) {
            reasons.push("10Y breakeven rising over ~3 months".into());
        }
        reasons.push(format!("policy stance = {policy_stance} (not easing)"));
    }
    // YELLOW: persistent above-target (core PCE 3m & 6m >= 3.0% + core CPI hot).
    else if ge(pce3, 3.0) && ge(pce6, 3.0) && core_cpi_hot_3_or_6 {
        status = Status::Yellow;
        reasons.push("core PCE 3-mo & 6-mo annualized both >= 3.0%".into());
        reasons.push("core CPI also >= 3.0% (3-mo or 6-mo)".into());
        // Note the boundary: hot on levels but RED context clause not met.
        // Do NOT port the unreachable `if not core_cpi_hot_3_or_6` dead branch.
        if ge(pce3, 3.5) && ge(pce6, 3.5) && !context_not_easing {
            let be_txt = if let Some(b) = be_last {
                format!(
                    "breakeven {:.2} < 2.5% and not clearly rising",
                    b.value
                )
            } else {
                "breakeven unavailable this run".into()
            };
            reasons.push(format!(
                "levels reach RED but context clause not met \
                 ({be_txt}, or stance easing) — human resolves via policy_stance"
            ));
        }
    }
    // WATCH: one hot print / mixed.
    else if ge(pce3, 2.5) || (core_cpi_hot_3_or_6 && !ge(pce3, 3.0)) {
        status = Status::Watch;
        if ge(pce3, 2.5) {
            reasons.push(
                "core PCE 3-mo annualized >= 2.5% but 6-mo not confirming".into(),
            );
        }
        if core_cpi_hot_3_or_6 {
            reasons.push("core CPI hot while core PCE not yet confirming".into());
        }
    }
    // OK: not confirmed, or trend falling.
    else {
        status = Status::Ok;
        if pce_falling {
            reasons.push("core PCE 3-mo pace below 6-mo pace (disinflating)".into());
        } else {
            reasons.push("core PCE 3-mo < 2.5% and 6-mo < 2.75%".into());
        }
    }

    let details = Details {
        core_pce_day: core_pce.last().unwrap().day.clone(),
        pce3,
        pce6,
        cpi3,
        cpi6,
        breakeven: be_last.map(|b| b.value),
        breakeven_day: be_last.map(|b| b.day.clone()),
        breakeven_rising: be_rising,
        policy_stance: policy_stance.to_string(),
        core_pce_obs: core_pce.len(),
        reasons,
    };
    (status, details)
}
