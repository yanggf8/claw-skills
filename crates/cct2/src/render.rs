//! The delivered report.

use crate::merge::{Agreement, Row};

pub fn fmt_sentiment(s: &str) -> String {
    match s {
        "bullish" => "看漲 🟢".to_string(),
        "bearish" => "看跌 🔴".to_string(),
        "neutral" => "中性 ⚪".to_string(),
        other => other.to_string(),
    }
}

/// Confidence as a whole percent.
///
/// Truncating, not rounding — `int(c * 100)` in the Python, so 0.789 shows 78%.
/// Kept: a confidence is already an estimate, and rounding 0.789 up to 79%
/// would be this program adding precision the model did not claim.
pub fn fmt_conf(c: f64) -> String {
    format!("{}%", (c * 100.0) as i64)
}

/// Truncate to `n` characters, counting characters rather than bytes.
///
/// The Python sliced with `reason[:80]`, which counts characters. Slicing a
/// Rust `&str` by byte index would panic mid-codepoint on the Chinese these
/// reasons are written in.
fn clip(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

pub fn format_report(rows: &[Row], mode: &str, ticker_count: usize, date: &str) -> String {
    let label = if mode == "pre-market" {
        "盤前報告"
    } else {
        "收盤報告"
    };
    let mut lines = vec![format!("📊 CCT2 {label}｜{date}"), String::new()];

    if rows.is_empty() {
        lines.push("⚠️ 無法取得任何分析結果".to_string());
        return lines.join("\n");
    }

    let of = |a: Agreement| rows.iter().filter(move |r| r.agreement == a);
    let consensus: Vec<&Row> = of(Agreement::Consensus).collect();
    let diverged: Vec<&Row> = of(Agreement::Diverged).collect();
    let solo: Vec<&Row> = of(Agreement::Solo).collect();

    if !consensus.is_empty() {
        lines.push("🎯 共識訊號".to_string());
        for r in &consensus {
            let mut line = format!(
                "  • {} {} {}",
                r.ticker,
                fmt_sentiment(&r.sentiment),
                fmt_conf(r.confidence)
            );
            if !r.reason.is_empty() {
                line.push_str(&format!(" — {}", clip(&r.reason, 80)));
            }
            lines.push(line);
        }
    }

    if !diverged.is_empty() {
        if !consensus.is_empty() {
            lines.push(String::new());
        }
        lines.push("⚠️ 分歧訊號".to_string());
        for r in &diverged {
            lines.push(format!("  • {}", r.ticker));
            lines.push(format!(
                "      主模型：{} {} — {}",
                fmt_sentiment(&r.primary.sentiment),
                fmt_conf(r.primary.confidence),
                clip(&r.primary.reason, 70)
            ));
            lines.push(format!(
                "      備用模型：{} {} — {}",
                fmt_sentiment(&r.backup.sentiment),
                fmt_conf(r.backup.confidence),
                clip(&r.backup.reason, 70)
            ));
        }
    }

    if !solo.is_empty() {
        if !consensus.is_empty() || !diverged.is_empty() {
            lines.push(String::new());
        }
        // This section was unreachable in the Python. A single-model answer was
        // filed under 共識訊號 instead, which told the reader two models had
        // agreed when only one had spoken.
        lines.push("📊 單一模型".to_string());
        for r in &solo {
            let which = if r.primary.is_present() {
                "主模型"
            } else {
                "備用模型"
            };
            let mut line = format!(
                "  • {} {} {}（僅{}）",
                r.ticker,
                fmt_sentiment(&r.sentiment),
                fmt_conf(r.confidence),
                which
            );
            if !r.reason.is_empty() {
                line.push_str(&format!(" — {}", clip(&r.reason, 80)));
            }
            lines.push(line);
        }
    }

    lines.push(String::new());
    // Say what actually happened. "雙模型對照" printed unconditionally, so a run
    // where the backup never answered still claimed a two-model comparison.
    let compared = rows
        .iter()
        .filter(|r| r.agreement != Agreement::Solo)
        .count();
    let footer = if compared == rows.len() {
        format!("分析標的：{ticker_count} 支｜雙模型對照")
    } else if compared == 0 {
        format!("分析標的：{ticker_count} 支｜單一模型回應")
    } else {
        format!(
            "分析標的：{ticker_count} 支｜{compared} 支雙模型對照，{} 支僅單一模型",
            rows.len() - compared
        )
    };
    lines.push(footer);
    lines.join("\n")
}
