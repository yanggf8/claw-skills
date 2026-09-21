//! The delivered report.

use crate::merge::{Agreement, Row};
use crate::review::{tally, Outcome, Reviewed, NEUTRAL_BAND_PCT};

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

/// Signed percent with two decimals, e.g. `+1.24%`.
fn fmt_pct(p: f64) -> String {
    format!("{}{:.2}%", if p >= 0.0 { "+" } else { "" }, p)
}

/// The morning's calls, and what the close did to them.
///
/// Rendered above the day's own analysis because it is the part that can be
/// checked. Returns no lines at all when there is no journal to review — an
/// empty section would read as "we predicted nothing", which is a different
/// claim from "no record was kept".
pub fn format_review(reviewed: &[Reviewed], made_at: &str) -> Vec<String> {
    if reviewed.is_empty() {
        return Vec::new();
    }
    let (hits, scored) = tally(reviewed);
    let mut lines = vec![if made_at.is_empty() {
        "🔁 盤前預測覆盤".to_string()
    } else {
        format!("🔁 盤前預測覆盤（{made_at} 的判斷）")
    }];

    for r in reviewed {
        let mark = match r.outcome {
            Outcome::Hit => "✅",
            Outcome::Miss => "❌",
            Outcome::Unscored => "➖",
        };
        let moved = match r.pct_change {
            Some(p) => format!("實際 {}", fmt_pct(p)),
            None => "實際 無報價".to_string(),
        };
        lines.push(format!(
            "  {mark} {} 盤前{} {} → {}",
            r.ticker,
            fmt_sentiment(&r.predicted),
            fmt_conf(r.confidence),
            moved
        ));
    }

    lines.push(String::new());
    // The band is printed because the verdicts are meaningless without it: the
    // same close is a hit or a miss depending on where the neutral zone sits.
    let unscored = reviewed.len() - scored;
    let mut foot = if scored == 0 {
        "  盤前預測無一可評分（缺少報價或方向）".to_string()
    } else {
        format!("  命中 {hits}/{scored}｜判定門檻 ±{NEUTRAL_BAND_PCT}%")
    };
    if unscored > 0 && scored > 0 {
        foot.push_str(&format!("，{unscored} 支無法評分"));
    }
    lines.push(foot);
    lines
}

/// Everything the header and the review section need, beside the rows.
///
/// A struct rather than a parameter list: the four strings are all `&str` and
/// three of them are optional-by-emptiness, so positional arguments could be
/// transposed silently — `date` and `market_time` would still compile swapped.
#[derive(Debug, Clone, Copy, Default)]
pub struct ReportContext<'a> {
    pub mode: &'a str,
    pub ticker_count: usize,
    /// The ET trading day.
    pub date: &'a str,
    /// Market-time stamp, e.g. `16:10 EDT`. Empty renders no stamp.
    pub market_time: &'a str,
    pub review: &'a [Reviewed],
    /// When the reviewed predictions were made, in market time.
    pub review_made_at: &'a str,
}

pub fn format_report(rows: &[Row], ctx: &ReportContext) -> String {
    let ReportContext {
        mode,
        ticker_count,
        date,
        market_time,
        review,
        review_made_at,
    } = *ctx;
    let label = if mode == "pre-market" {
        "盤前報告"
    } else {
        "收盤報告"
    };
    // The date is the ET trading day and the stamp is market time, so the
    // header names the session it belongs to rather than leaving the reader to
    // infer it from when the message arrived in their own zone.
    let head = if market_time.is_empty() {
        format!("📊 CCT2 {label}｜{date}")
    } else {
        format!("📊 CCT2 {label}｜{date} {market_time}")
    };
    let mut lines = vec![head, String::new()];

    let review_lines = format_review(review, review_made_at);
    if !review_lines.is_empty() {
        lines.extend(review_lines);
        lines.push(String::new());
        lines.push("────────────".to_string());
        lines.push(String::new());
    }

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
