//! Fed-braking watch — the mechanical half of the doc-191 composite condition.
//!
//! Composite: 「泡沫語境持續(人讀) × EFFR 6 個月 +100bp(機械)」. This module
//! and the `--mode braking` run automate ONLY the mechanical half; the
//! bubble-context half stays human-read and is never evaluated here.
//!
//! Two series, two deliberately different roles (Codex review 2026-10-02):
//! - FEDFUNDS (monthly EFFR average) is the RULE series. The reference line is
//!   month-index arithmetic — latest minus the observation six months earlier.
//!   A daily-series approximation is NOT the same measure and is never
//!   presented as one.
//! - DFEDTARU (daily target-range upper bound; DFEDTAR is discontinued 2008,
//!   DFEDTARL is the lower bound) is the TIMELINESS series. Daily observation
//!   surfaces a Fed move weeks before the monthly average can; on its own it
//!   says nothing about the rule.
//!
//! The change detector keeps a persisted cursor (last delivered date+value):
//! a first run is a silent baseline (history is not news), an unchanged
//! target is a no-op, and the cursor advances only after a successful
//! delivery — a failed delivery must re-report on the next run instead of
//! swallowing the change.
//!
//! Signal-only: rendered output is arithmetic plus a stated reference line.
//! No status vocabulary (OK/WATCH/YELLOW/RED), no verdict wording, no advice.

use crate::analysis::Obs;

/// Rule series: monthly EFFR average (FRED FEDFUNDS).
pub const RULE_SERIES: &str = "FEDFUNDS";
/// Timeliness series: daily target-range upper bound (FRED DFEDTARU).
pub const TARGET_SERIES: &str = "DFEDTARU";
/// Reference line for the composite condition: bp rise over six months.
pub const RULE_BP: f64 = 100.0;

/// Month-index rule arithmetic on FEDFUNDS. `rise_bp` is rounded to an integer
/// basis point figure; nothing here is compared against `RULE_BP` — the reader
/// compares, the tool does not.
#[derive(Debug, Clone, PartialEq)]
pub struct BrakingRule {
    pub latest_month: String,
    pub latest: f64,
    pub earlier_month: String,
    pub earlier: f64,
    pub rise_bp: i64,
}

/// Persisted cursor: the DFEDTARU (date, value) last successfully delivered.
#[derive(Debug, Clone, PartialEq)]
pub struct Cursor {
    pub date: String,
    pub value: f64,
}

/// Outcome of comparing the fetched series against the cursor.
#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    /// No cursor on disk: record the latest (date, value) and deliver nothing.
    Baseline { cursor: Cursor },
    /// Nothing new since the cursor.
    Unchanged,
    /// The target moved (or moved and round-tripped) since the cursor.
    /// `intermediate` are the distinct water levels passed en route, excluding
    /// levels equal to the cursor value (those are the old level, not news).
    Changed {
        from: (String, f64),
        to: (String, f64),
        intermediate: Vec<(String, f64)>,
    },
}

fn month_of(day: &str) -> String {
    day.get(..7).unwrap_or(day).to_string()
}

fn neq(a: f64, b: f64) -> bool {
    (a - b).abs() > 1e-9
}

/// Rule math over monthly FEDFUNDS observations: latest vs the observation
/// six months earlier (index len-7). None when fewer than seven months are
/// available — insufficient coverage is reported, never narrowed.
pub fn rule_math(obs: &[Obs]) -> Option<BrakingRule> {
    if obs.len() < 7 {
        return None;
    }
    let latest = obs.last()?;
    let earlier = &obs[obs.len() - 7];
    Some(BrakingRule {
        latest_month: month_of(&latest.day),
        latest: latest.value,
        earlier_month: month_of(&earlier.day),
        earlier: earlier.value,
        rise_bp: ((latest.value - earlier.value) * 100.0).round() as i64,
    })
}

/// Compare the fetched DFEDTARU series against the cursor.
///
/// The scan window is every observation on or after the cursor date —
/// inclusive so a same-date revision of the cursor value still surfaces.
/// Observations strictly before the cursor date are history the cursor
/// already covered; if the data has not reached the cursor date the window
/// is empty and the run reads as unchanged.
pub fn detect(cursor: Option<&Cursor>, obs: &[Obs]) -> Decision {
    let Some(last) = obs.last() else {
        // No observations at all: nothing to report either way.
        return Decision::Unchanged;
    };
    let Some(c) = cursor else {
        return Decision::Baseline {
            cursor: Cursor {
                date: last.day.clone(),
                value: last.value,
            },
        };
    };

    let window: &[Obs] = match obs.iter().position(|o| o.day.as_str() >= c.date.as_str()) {
        Some(p) => &obs[p..],
        None => return Decision::Unchanged,
    };

    // Compress the window into runs of equal values; each run's first date
    // stands for the whole run.
    let mut runs: Vec<(String, f64)> = Vec::new();
    for o in window {
        match runs.last_mut() {
            Some(last_run) if !neq(last_run.1, o.value) => {}
            _ => runs.push((o.day.clone(), o.value)),
        }
    }
    let to = window.last().expect("window is non-empty");
    // Every run except the final one is an intermediate water level; runs
    // equal to the cursor value are the old level revisited, not news.
    let intermediate: Vec<(String, f64)> = runs[..runs.len() - 1]
        .iter()
        .filter(|(_, v)| neq(*v, c.value))
        .cloned()
        .collect();

    if neq(to.value, c.value) || !intermediate.is_empty() {
        Decision::Changed {
            from: (c.date.clone(), c.value),
            to: (to.day.clone(), to.value),
            intermediate,
        }
    } else {
        Decision::Unchanged
    }
}

/// Monthly-report section. Empty string when the rule series was unavailable
/// (the caller omits the section entirely — a failed add-on must not degrade
/// the inflation skill status).
pub fn render_monthly_section(rule: Option<&BrakingRule>, fedfunds_count: usize, target: Option<&Obs>) -> String {
    if rule.is_none() && fedfunds_count == 0 {
        return String::new();
    }
    let mut lines = vec!["──── Fed braking(規則計算,不判讀)────".to_string()];
    match rule {
        Some(r) => {
            lines.push(format!("FEDFUNDS {} 月均:{:.2}%", r.latest_month, r.latest));
            lines.push(format!(
                "六個月前({}):{:.2}% → 期間 {:+}bp(參考線:+{}bp)",
                r.earlier_month, r.earlier, r.rise_bp, RULE_BP
            ));
        }
        None => {
            lines.push(format!("FEDFUNDS 月觀察值不足({}/7)— 規則計算待補", fedfunds_count));
        }
    }
    match target {
        Some(t) => lines.push(format!("目標區間上限 DFEDTARU:{:.2}%({})", t.value, t.day)),
        None => lines.push("目標區間上限 DFEDTARU:n/a".to_string()),
    }
    lines.push("泡沫語境由人判讀;本工具不判讀。".to_string());
    lines.join("\n")
}

/// Daily-change delivery body (parse_mode: None — plain text, no padding).
pub fn render_daily_message(
    from: &(String, f64),
    to: &(String, f64),
    intermediate: &[(String, f64)],
    rule: Option<&BrakingRule>,
) -> String {
    let bp = ((to.1 - from.1) * 100.0).round() as i64;
    let mut lines = vec![
        "FED BRAKING".to_string(),
        format!(
            "目標區間上限(DFEDTARU)變動:{:.2}%({}) → {:.2}%({})",
            from.1, from.0, to.1, to.0
        ),
        format!("變動量:{:+}bp", bp),
    ];
    if !intermediate.is_empty() {
        let items: Vec<String> = intermediate
            .iter()
            .map(|(d, v)| format!("{:.2}%({})", v, d))
            .collect();
        lines.push(format!("期間另有水位:{}", items.join("、")));
    }
    match rule {
        Some(r) => lines.push(format!(
            "FEDFUNDS {} 月均:{:.2}%;六個月前({}):{:.2}% → 期間 {:+}bp(參考線:+{}bp)",
            r.latest_month, r.latest, r.earlier_month, r.earlier, r.rise_bp, RULE_BP
        )),
        None => lines.push("FEDFUNDS:n/a(月規則計算待補)".to_string()),
    }
    lines.push("泡沫語境由人判讀;本工具不判讀。".to_string());
    lines.join("\n")
}

/// Quiet line for a no-op run (stdout only, no delivery).
pub fn no_op_line(latest: Option<&Obs>) -> String {
    match latest {
        Some(t) => format!(
            "fed-braking no-op:目標區間上限未變 {:.2}%({})",
            t.value, t.day
        ),
        None => "fed-braking no-op:無 DFEDTARU 資料".to_string(),
    }
}

/// Quiet line for the first-run baseline (cursor recorded, nothing delivered).
pub fn baseline_line(cursor: &Cursor) -> String {
    format!(
        "fed-braking baseline:初次基準 {:.2}%({});已記錄 cursor,本次不遞送",
        cursor.value, cursor.date
    )
}

// Cursor persistence is two stringly fields; the JSON never becomes a data
// model (agent-first / no-JSON outside adapters).

impl Cursor {
    /// Serialize for the cursor file. Hand-rolled so serde never becomes a
    /// public API of this crate.
    pub fn to_json(&self) -> String {
        format!("{{\"date\":{},\"value\":{}}}", json_str(&self.date), self.value)
    }

    /// Parse a cursor file. None on any malformation — the caller refuses
    /// rather than silently re-baselining.
    pub fn from_json(text: &str) -> Option<Cursor> {
        let v: serde_json::Value = serde_json::from_str(text).ok()?;
        Some(Cursor {
            date: v.get("date")?.as_str()?.to_string(),
            value: v.get("value")?.as_f64()?,
        })
    }
}

fn json_str(s: &str) -> String {
    serde_json::Value::String(s.to_string()).to_string()
}
