//! Message and record-line rendering for oilcon.
//! Line-for-line translation of the format helpers in oilcon/scripts/run.py.
//! Timestamps are parameters — this layer never reads a clock.

use crate::analysis::{
    classify_oil_trend, compute_change_pct, compute_extremes, ma_rising, moving_average,
    pct_below_60d_high, Extremes,
};
// Types live in snapshot.rs (assembly owns them); re-export so existing
// `render::Snapshot` / `render::SymbolSnapshot` paths keep working.
pub use crate::snapshot::{Snapshot, SymbolSnapshot};

/// Refusal / requirement error whose message is part of the stderr contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatError {
    message: &'static str,
}

impl FormatError {
    pub fn as_str(&self) -> &'static str {
        self.message
    }
}

impl std::fmt::Display for FormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message)
    }
}

impl std::error::Error for FormatError {}

pub fn fmt_price(value: f64) -> String {
    format!("${value:.2}")
}

pub fn fmt_pct(value: f64) -> String {
    let prefix = if value >= 0.0 { "+" } else { "" };
    format!("{prefix}{value:.1}%")
}

fn sign(value: f64) -> i32 {
    if value > 0.0 {
        1
    } else if value < 0.0 {
        -1
    } else {
        0
    }
}

/// En dash (U+2013) for flat; ✓ / ✗ when both sides have a non-zero change.
fn confirmation_mark(confirm_change: f64, wti_change: f64) -> &'static str {
    if confirm_change == 0.0 || wti_change == 0.0 {
        "–"
    } else if sign(confirm_change) == sign(wti_change) {
        "✓"
    } else {
        "✗"
    }
}

pub fn format_wti_line(symbol_snapshot: &SymbolSnapshot) -> Result<(String, Extremes), FormatError> {
    let rows = symbol_snapshot
        .rows
        .as_deref()
        .ok_or(FormatError {
            message: "WTI rows are required",
        })?;

    let wti = compute_extremes(rows);
    let mut line = format!(
        "WTI: {} ({})",
        fmt_price(wti.current_close),
        fmt_pct(wti.today_change_pct)
    );
    if symbol_snapshot.stale {
        line.push_str(" (stale)");
    }
    Ok((line, wti))
}

pub fn format_confirmation_segment(
    label: &str,
    symbol_snapshot: &SymbolSnapshot,
    wti_change: f64,
) -> String {
    let Some(rows) = symbol_snapshot.rows.as_deref() else {
        return format!("{label} n/a");
    };

    let change = compute_change_pct(rows);
    let mark = confirmation_mark(change, wti_change);
    let mut segment = format!("{label} {mark} ({})", fmt_pct(change));
    if symbol_snapshot.stale {
        segment.push_str(" (stale)");
    }
    segment
}

/// Render the deliver-mode message. `now` is the `cst_now()` string (no seconds).
/// Returns `(message, status)` where status is `"ok"` or `"degraded"`.
pub fn format_message(snapshot: &Snapshot, now: &str) -> (String, String) {
    let (wti_line, wti) = format_wti_line(&snapshot.wti).expect("WTI rows are required");
    let brent_segment = format_confirmation_segment(
        "Brent",
        &snapshot.brent,
        wti.today_change_pct,
    );
    let ho_segment = format_confirmation_segment("HO", &snapshot.ho, wti.today_change_pct);

    let mut lines: Vec<String> = vec!["🛢️ OILCON 情報".into()];
    let mut status = "ok".to_string();
    if let Some(ref warning) = snapshot.warning {
        // Say what is affected, not a tag. This line rides on an otherwise
        // complete report, so the reader needs to know which part to distrust.
        lines.push(format!("⚠ 這份不完整:{warning}"));
        status = "degraded".to_string();
    }

    lines.push(wti_line);
    lines.push(format!(
        "  高 {} ({}, {}日前, {})",
        fmt_price(wti.high_close),
        wti.high_day,
        wti.days_since_high,
        fmt_pct(wti.distance_from_high_pct)
    ));
    lines.push(format!(
        "  低 {} ({}, {}日前, {} 離低點)",
        fmt_price(wti.low_close),
        wti.low_day,
        wti.days_since_low,
        fmt_pct(wti.distance_off_low_pct)
    ));
    lines.push(format!("確認：{brent_segment}   {ho_segment}"));
    lines.push(format!("更新：{now}"));

    // OIL-TREND line: threshold is 50, not classify_oil_trend's 70.
    // Display comparator is `>=` (above/below), which disagrees with the
    // classifier's strict `>` at exact equality — port as-is.
    if let Some(ref rows) = snapshot.wti.rows {
        if rows.len() >= 50 {
            let state = classify_oil_trend(rows);
            let ma50 = moving_average(rows, 50).expect("rows.len() >= 50");
            let current_price = wti.current_close;
            let above_below = if current_price >= ma50 {
                "above"
            } else {
                "below"
            };
            let ma_dir = if ma_rising(rows, 50, 20) {
                "rising"
            } else {
                "flat/down"
            };
            let pct = pct_below_60d_high(rows);
            lines.push(format!(
                "OIL-TREND: {state} (WTI {current_price:.2}, {above_below} 50MA {ma50:.2}, 50MA {ma_dir}, {pct:.1}% vs 60d-high)"
            ));
        }
    }

    (lines.join("\n"), status)
}

/// Render a history-log line. `now` is `cst_now(with_seconds=True)`.
/// Refuses three separate conditions — each with its own message — as `Err`.
pub fn format_record_line(snapshot: &Snapshot, now: &str) -> Result<String, FormatError> {
    if snapshot.warning.is_some() {
        return Err(FormatError {
            message: "record mode requires fresh data",
        });
    }

    if snapshot.wti.stale || snapshot.brent.stale || snapshot.ho.stale {
        return Err(FormatError {
            message: "record mode requires non-stale data",
        });
    }

    let wti_rows = snapshot.wti.rows.as_deref();
    let brent_rows = snapshot.brent.rows.as_deref();
    let ho_rows = snapshot.ho.rows.as_deref();
    if wti_rows.is_none() || brent_rows.is_none() || ho_rows.is_none() {
        return Err(FormatError {
            message: "record mode requires complete confirmation data",
        });
    }
    let wti_rows = wti_rows.unwrap();
    let brent_rows = brent_rows.unwrap();
    let ho_rows = ho_rows.unwrap();

    let wti = compute_extremes(wti_rows);
    let brent_change = compute_change_pct(brent_rows);
    let ho_change = compute_change_pct(ho_rows);
    Ok(format!(
        "{now}  \
         WTI {:.2}  \
         high {:.2}@{} ({})  \
         low {:.2}@{} ({})  \
         BZ {} HO {}",
        wti.current_close,
        wti.high_close,
        wti.high_day,
        fmt_pct(wti.distance_from_high_pct),
        wti.low_close,
        wti.low_day,
        fmt_pct(wti.distance_off_low_pct),
        fmt_pct(brent_change),
        fmt_pct(ho_change),
    ))
}
