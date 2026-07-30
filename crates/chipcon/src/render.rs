//! Message and history-line rendering.
//! Line-by-line translation of chipcon/scripts/run.py
//! `fmt_price`, `fmt_pct`, `format_message`, `record_line`.
//!
//! Structural change vs Python: `record_line` takes the timestamp as a
//! parameter (instead of calling the clock) so the line can be tested.

use crate::analysis::{Details, Status};
use crate::config::Config;

/// Skill status returned to nullclaw — not the market classification.
pub type SkillStatus = &'static str;

/// Status strings must match Python exactly (rendered into message + record line).
fn status_str(status: Status) -> &'static str {
    match status {
        Status::Ok => "OK",
        Status::Yellow => "YELLOW",
        Status::Orange => "ORANGE",
        Status::Red => "RED",
        Status::ProfitProtect => "PROFIT_PROTECT",
        Status::InsufficientHistory => "INSUFFICIENT_HISTORY",
    }
}

pub fn fmt_price(value: Option<f64>) -> String {
    match value {
        None => "n/a".into(),
        Some(v) => format!("{v:.2}"),
    }
}

pub fn fmt_pct(value: Option<f64>) -> String {
    match value {
        None => "n/a".into(),
        Some(v) => {
            let prefix = if v >= 0.0 { "+" } else { "" };
            format!("{prefix}{v:.1}%")
        }
    }
}

/// Three-valued rising20 display, matching Python:
///   "rising" if truthy, "flat/down" if is False, "n/a" if None.
fn ma20_dir(rising20: Option<bool>) -> &'static str {
    match rising20 {
        Some(true) => "rising",
        Some(false) => "flat/down",
        None => "n/a",
    }
}

/// Format the Telegram body and the *skill* status ("ok" / "degraded").
///
/// Skill status is independent of classification: a RED with no warning is
/// still "ok". A warning (any classification) is "degraded".
pub fn format_message(
    status: Status,
    details: &Details,
    cfg: &Config,
    warning: Option<&str>,
) -> (String, SkillStatus) {
    let mut lines: Vec<String> = vec!["💾 CHIPCON 情報".into()];
    let mut skill_status: SkillStatus = "ok";
    if let Some(w) = warning {
        lines.push(format!("[WARN: {w}]"));
        skill_status = "degraded";
    }
    lines.push(format!("狀態：{}", status_str(status)));

    if status == Status::InsufficientHistory {
        lines.push(format!(
            "SMH history rows: {} / 20 needed",
            details.rows
        ));
    } else {
        let dir = ma20_dir(details.rising20);
        // Python: fmt_price(details['current']) — current is always a float.
        lines.push(format!(
            "SMH：{} ({})",
            fmt_price(Some(details.current)),
            details.day
        ));
        lines.push(format!(
            "20DMA：{} ({} vs 20DMA, {dir})",
            fmt_price(details.ma20),
            fmt_pct(details.distance20)
        ));
        lines.push(format!(
            "50DMA：{} ({} vs 50DMA)",
            fmt_price(details.ma50),
            fmt_pct(details.distance50)
        ));
        lines.push(format!(
            "5日：SMH {} / QQQ {} / SOXX {}",
            fmt_pct(details.smh5),
            fmt_pct(details.qqq5),
            fmt_pct(details.soxx5)
        ));
        lines.push(format!(
            "相對：SMH-QQQ {} / SMH-SOXX {}",
            fmt_pct(details.rel_qqq5),
            fmt_pct(details.rel_soxx5)
        ));
        lines.push(format!("連跌：{} 日", details.down_days));
        if !details.reasons.is_empty() {
            lines.push("原因：".into());
            for reason in &details.reasons {
                lines.push(format!("- {reason}"));
            }
        } else {
            lines.push("原因：trend intact".into());
        }
    }

    lines.push(String::new());
    lines.push("事件人工檢查：".into());
    // Python: for event in cfg.get("manual_events", default_events())
    // Config always carries the field (load_config setdefaults it).
    for event in &cfg.manual_events {
        lines.push(format!("- {event}"));
    }

    lines.push(String::new());
    lines.push("SIGNAL-ONLY：這是動能觀測信號，不是交易指令。".into());
    (lines.join("\n"), skill_status)
}

/// History log line. `now` is injected so the line is unit-testable; main
/// passes the real CST timestamp string.
pub fn record_line(
    status: Status,
    details: &Details,
    warning: Option<&str>,
    now: &str,
) -> String {
    let warn = warning.unwrap_or("-");
    if status == Status::InsufficientHistory {
        return format!(
            "{now} CHIPCON {} rows={} warning={warn}",
            status_str(status),
            details.rows
        );
    }
    format!(
        "{now} CHIPCON {} SMH={:.2} ma20={} ma50={} rel_qqq5={} down={} warning={warn}",
        status_str(status),
        details.current,
        fmt_price(details.ma20),
        fmt_price(details.ma50),
        fmt_pct(details.rel_qqq5),
        details.down_days,
    )
}
