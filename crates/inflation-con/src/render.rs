//! Message and history-line rendering.
//! Line-by-line translation of inflation-con/scripts/run.py
//! `fmt_pct`, `fmt_num`, `format_message`, `record_line`.
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
        Status::Watch => "WATCH",
        Status::Yellow => "YELLOW",
        Status::Red => "RED",
        Status::InsufficientData => "INSUFFICIENT_DATA",
    }
}

/// run.py:242-246
pub fn fmt_pct(v: Option<f64>) -> String {
    match v {
        None => "n/a".into(),
        Some(x) => {
            let sign = if x >= 0.0 { "+" } else { "" };
            format!("{sign}{x:.2}%")
        }
    }
}

/// run.py:249-250
pub fn fmt_num(v: Option<f64>) -> String {
    match v {
        None => "n/a".into(),
        Some(x) => format!("{x:.2}"),
    }
}

/// Three-valued breakeven_rising display, matching run.py:264-265:
///   "rising" if True, "flat/down" if False, "n/a" if None.
fn rising_str(rising: Option<bool>) -> &'static str {
    match rising {
        Some(true) => "rising",
        Some(false) => "flat/down",
        None => "n/a",
    }
}

/// Format the Telegram body and the *skill* status ("ok" / "degraded").
///
/// Skill status is independent of classification: a RED with no warning is
/// still "ok". A warning (any classification) is "degraded".
///
/// `cfg` is accepted for signature parity with Python (run.py:253) but is
/// unused — same as the Python body.
pub fn format_message(
    status: Status,
    details: &Details,
    _cfg: &Config,
    warning: Option<&str>,
) -> (String, SkillStatus) {
    let mut lines: Vec<String> = vec!["📈 INFLATION-CON".into()];
    let mut skill_status: SkillStatus = "ok";
    if let Some(w) = warning {
        lines.push(format!("[WARN: {w}]"));
        skill_status = "degraded";
    }
    lines.push(format!("狀態：{}", status_str(status)));

    if status == Status::InsufficientData {
        // run.py:261-262 — skip the whole indicator block
        lines.push(format!(
            "core PCE obs: {} / 7 needed",
            details.core_pce_obs
        ));
    } else {
        // run.py:264-274
        let r = rising_str(details.breakeven_rising);
        // Python f-string prints None as "None" when breakeven_day is missing.
        let be_day = details
            .breakeven_day
            .as_deref()
            .unwrap_or("None");
        lines.push(format!(
            "核心PCE ({})：3mo {} / 6mo {} 年化",
            details.core_pce_day,
            fmt_pct(details.pce3),
            fmt_pct(details.pce6),
        ));
        lines.push(format!(
            "核心CPI：3mo {} / 6mo {} 年化",
            fmt_pct(details.cpi3),
            fmt_pct(details.cpi6),
        ));
        lines.push(format!(
            "10Y breakeven：{} ({be_day}, {r})",
            fmt_num(details.breakeven),
        ));
        lines.push(format!(
            "FOMC 立場 (manual)：{}",
            details.policy_stance
        ));
        if !details.reasons.is_empty() {
            lines.push("依據：".into());
            for reason in &details.reasons {
                lines.push(format!("- {reason}"));
            }
        }
    }

    // run.py:276-283 — always rendered, including on INSUFFICIENT_DATA
    lines.push(String::new());
    lines.push("人工檢查（不入演算法）：".into());
    lines.push("- 最新 CPI (~月中) / PCE (~月底) release 是否已納入".into());
    lines.push(
        "- FOMC 立場（restrictive / neutral / easing / unclear）是否需更新 config".into(),
    );
    lines.push(
        "- 能源價格是否推高 headline 但 core 未跟進（headline 只是 context）".into(),
    );
    lines.push(String::new());
    lines.push("SIGNAL-ONLY：這是通膨『確認證據』分級，不是交易指令。".into());
    lines.push(
        "RED = 進入 review（是否加通膨對沖？IEF gate？），由人決定並記 decision add。".into(),
    );
    (lines.join("\n"), skill_status)
}

/// History log line. `now` is injected so the line is unit-testable; main
/// will pass the real CST timestamp string.
///
/// run.py:287-297 has two shapes; both end with `warning=` and a dash when
/// there is none.
pub fn record_line(
    status: Status,
    details: &Details,
    warning: Option<&str>,
    now: &str,
) -> String {
    let warn = warning.unwrap_or("-");
    if status == Status::InsufficientData {
        return format!(
            "{now} INFLATION-CON {} obs={} warning={warn}",
            status_str(status),
            details.core_pce_obs,
        );
    }
    format!(
        "{now} INFLATION-CON {} \
         pce3={} pce6={} cpi3={} cpi6={} \
         be={} stance={} warning={warn}",
        status_str(status),
        fmt_pct(details.pce3),
        fmt_pct(details.pce6),
        fmt_pct(details.cpi3),
        fmt_pct(details.cpi6),
        fmt_num(details.breakeven),
        details.policy_stance,
    )
}
