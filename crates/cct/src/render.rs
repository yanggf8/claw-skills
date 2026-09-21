//! Rendering the four reports.

use crate::freshness::pre_market_freshness;
use jiff::civil::Date;

pub fn fmt_sentiment(s: &str) -> String {
    match s.to_lowercase().as_str() {
        "bullish" => "看漲 🟢".into(),
        "bearish" => "看跌 🔴".into(),
        "neutral" => "中性 ⚪".into(),
        _ => s.to_string(),
    }
}

/// Clip to `n` characters. Python's `s[:80]` counts characters; byte-slicing a
/// Rust `&str` would panic mid-codepoint on the Chinese these carry.
fn clip(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

fn str_of<'a>(v: &'a serde_json::Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(|x| x.as_str()).filter(|s| !s.is_empty())
}

fn pct(v: Option<&serde_json::Value>) -> i64 {
    (v.and_then(|x| x.as_f64()).unwrap_or(0.0) * 100.0) as i64
}

/// The pre-market report.
///
/// A stale payload renders its date and an explanation, and nothing else.
///
/// The header warning alone was not enough. On 2026-07-28 this report went out
/// carrying "⚠️ 資料已過期（50 天前）" and, four lines below it,
/// "AAPL 看漲 🟢 95%". Both statements were true and the combination was
/// misleading: 95% was the confidence the model held about 2026-06-08, and a
/// reader scanning a morning report takes a percentage next to a ticker as a
/// call on today. Restating those figures under a warning asks the reader to do
/// the discounting, which is the one thing a report should do for them.
pub fn format_pre_market(data: &serde_json::Value, today: Date) -> String {
    let fresh = pre_market_freshness(data, today);

    // The source date, never today's — a stale snapshot stamped with today's
    // date reads as current analysis, which is worse than no report at all.
    let mut header = format!(
        "📊 CCT 盤前報告｜{}",
        fresh.source_date.as_deref().unwrap_or("日期不明")
    );
    if fresh.is_stale {
        header += &match fresh.age_days {
            Some(n) => format!("  ⚠️ 資料已過期（{n} 天前）"),
            None => "  ⚠️ 資料已過期".to_string(),
        };
    }
    let mut lines = vec![header, String::new()];

    if fresh.is_stale {
        let when = fresh.source_date.as_deref().unwrap_or("日期不明");
        lines.push(format!(
            "上次分析：{when}，訊號與信心數值屬於當日市況，不適用於今日，故不列出。"
        ));
        lines.push("等待今日盤前分析完成。".into());
        return lines.join("\n");
    }

    let overall = data.get("overall_sentiment");
    let sentiment = overall
        .and_then(|o| str_of(o, "sentiment"))
        .or_else(|| str_of(data, "market_sentiment"));
    let confidence = overall
        .and_then(|o| o.get("confidence"))
        .or_else(|| data.get("confidence"));
    let analyzed = data
        .get("symbols_analyzed")
        .and_then(|v| v.as_i64())
        .filter(|n| *n != 0)
        .or_else(|| {
            data.get("trading_signals")
                .and_then(|v| v.as_object())
                .map(|o| o.len() as i64)
                .filter(|n| *n != 0)
        });

    if let Some(s) = sentiment {
        lines.push(format!(
            "市場情緒：{}（信心 {}%）",
            fmt_sentiment(s),
            pct(confidence)
        ));
    }
    if let Some(n) = analyzed {
        lines.push(format!("分析標的：{n} 支"));
    }
    lines.push(String::new());

    let signals = data
        .get("high_confidence_signals")
        .and_then(|v| v.as_array())
        .filter(|a| !a.is_empty());

    match signals {
        Some(list) => {
            lines.push("🎯 高信心訊號（≥70%）".into());
            for s in list.iter().take(8) {
                let sym = str_of(s, "symbol").unwrap_or("");
                let sent = fmt_sentiment(str_of(s, "sentiment").unwrap_or("neutral"));
                let conf = pct(s.get("confidence"));
                let reason = str_of(s, "reason").or_else(|| str_of(s, "reasoning"));
                let mut line = format!("  • {sym} {sent} {conf}%");
                if let Some(r) = reason {
                    line += &format!(" — {}", clip(r, 80));
                }
                lines.push(line);
            }
        }
        None => lines.push(match str_of(data, "message") {
            Some(m) => format!("⏳ {m}"),
            None => "今日尚無高信心訊號".into(),
        }),
    }

    lines.join("\n")
}

// ── end of day ───────────────────────────────────────────────────────────────

fn bias_label(b: &str) -> String {
    match b.to_lowercase().as_str() {
        "bullish" => "偏多 🟢".into(),
        "bearish" => "偏空 🔴".into(),
        "neutral" => "中性 ⚪".into(),
        _ => b.to_string(),
    }
}

/// The market session this report covers — never today's clock.
///
/// The scorecard payload carries no `date` (only the synthesised placeholder
/// does), so fall through the timestamps the snapshot does carry before
/// resorting to now(). Stamping a stale report with today's date is the exact
/// failure the pre-market gate exists to prevent; the same rule applies here.
pub fn eod_session_date(
    business_date: Option<&str>,
    data: &serde_json::Value,
    now: &str,
) -> String {
    // Stated beats inferred. Everything below is a guess chain that exists only
    // because nothing used to state the answer, and one of its links is worse
    // than a guess: `timestamp` is an ISO **UTC** instant, so its first ten
    // characters are a UTC day, not a business date — a session that closed on
    // 2026-08-06 ET renders as 2026-08-07. The envelope outranks even `date`,
    // because it is what the worker keyed its storage by, while the payload is
    // what it happened to render.
    if let Some(day) = business_date {
        return day.to_string();
    }
    for key in [
        "date",
        "_scheduled_date",
        "timestamp",
        "marketCloseTime",
        "generated_at",
    ] {
        if let Some(s) = data.get(key).and_then(|v| v.as_str()) {
            if s.chars().count() >= 10 {
                return s.chars().take(10).collect();
            }
        }
    }
    now.to_string()
}

/// First glyph of "↑ Expected" — the arrow, without the prose.
fn direction(text: &str) -> String {
    text.trim().chars().next().map(String::from).unwrap_or_default()
}

/// "↓ 0.6%" → "↓0.6%", so a row stays on one line on a phone.
fn tight(text: &str) -> String {
    text.split_whitespace().collect()
}

fn i(v: Option<&serde_json::Value>) -> i64 {
    v.and_then(|x| x.as_i64()).unwrap_or(0)
}

/// The live shape: a prediction scorecard, not a market summary.
fn scorecard_lines(data: &serde_json::Value) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();

    let correct = i(data.get("correctCalls"));
    let wrong = i(data.get("wrongCalls"));
    let graded = correct + wrong;
    let mut headline: Vec<String> = Vec::new();
    if let Some(g) = str_of(data, "modelGrade") {
        headline.push(format!("模型評級：{g}"));
    }
    if graded != 0 {
        headline.push(format!("高信心命中 {correct}/{graded}"));
    }
    if !headline.is_empty() {
        lines.push(headline.join("｜"));
    }

    let analyzed = data
        .get("symbols_analyzed")
        .and_then(|v| v.as_i64())
        .filter(|n| *n != 0)
        .or_else(|| data.get("totalSignals").and_then(|v| v.as_i64()).filter(|n| *n != 0));
    if let Some(n) = analyzed {
        lines.push(format!("分析標的：{n} 支"));
    }

    if let Some(breakdown) = data
        .get("signalBreakdown")
        .and_then(|v| v.as_array())
        .filter(|a| !a.is_empty())
    {
        lines.push(String::new());
        lines.push("🎯 訊號回顧".into());
        for s in breakdown.iter().take(8) {
            let conf = s.get("confidence").and_then(|c| c.as_f64()).unwrap_or(0.0);
            let mark = if s.get("correct").and_then(|c| c.as_bool()).unwrap_or(false) {
                "✓"
            } else {
                "✗"
            };
            let mut row = format!(
                "  • {} 預測{} 實際{}  {mark}",
                str_of(s, "ticker").unwrap_or(""),
                direction(str_of(s, "predicted").unwrap_or("")),
                tight(str_of(s, "actual").unwrap_or(""))
            );
            if conf != 0.0 {
                row += &format!(" {}%", conf as i64);
            }
            lines.push(row);
        }
    }

    if let Some(losers) = data
        .get("topLosers")
        .and_then(|v| v.as_array())
        .filter(|a| !a.is_empty())
    {
        let joined = losers
            .iter()
            .take(4)
            .map(|x| {
                format!(
                    "{} {}",
                    str_of(x, "ticker").unwrap_or(""),
                    str_of(x, "performance").unwrap_or("")
                )
                .trim()
                .to_string()
            })
            .collect::<Vec<_>>()
            .join("｜");
        lines.push(String::new());
        lines.push(format!("📉 落後：{joined}"));
    }

    if let Some(outlook) = data.get("tomorrowOutlook") {
        if let Some(bias) = str_of(outlook, "marketBias") {
            let mut detail: Vec<String> = Vec::new();
            if let Some(v) = str_of(outlook, "volatilityLevel") {
                detail.push(format!("波動 {v}"));
            }
            if let Some(c) = str_of(outlook, "confidenceLevel") {
                detail.push(format!("信心 {c}"));
            }
            let suffix = if detail.is_empty() {
                String::new()
            } else {
                format!("（{}）", detail.join("｜"))
            };
            lines.push(String::new());
            lines.push(format!("明日展望：{}{suffix}", bias_label(bias)));
            if let Some(f) = str_of(outlook, "keyFocus") {
                lines.push(format!("關注：{f}"));
            }
        }
    }

    lines
}

/// The end-of-day report.
///
/// Two shapes reach this: the live scorecard, and the placeholder the route
/// synthesises when it has no snapshot. Dispatch on which one arrived rather
/// than assuming — testing only for `daily_summary` is what reported degraded
/// on every genuine report from 2026-07-21.
pub fn format_eod(business_date: Option<&str>, data: &serde_json::Value, now: &str) -> String {
    let mut lines = vec![
        format!(
            "📊 CCT 收盤報告｜{}",
            eod_session_date(business_date, data, now)
        ),
        String::new(),
    ];

    // Dispatch on which shape arrived, explicitly. Deciding by "did the
    // scorecard renderer produce anything" is a heuristic that silently picks
    // the wrong branch for a thin scorecard.
    if data.get("signalBreakdown").is_some()
        || data.get("totalSignals").is_some()
        || data.get("modelGrade").is_some()
    {
        lines.extend(scorecard_lines(data));
        return lines.join("\n");
    }

    // Legacy / placeholder shape.
    let summary = data.get("daily_summary").cloned().unwrap_or(serde_json::json!({}));
    // `overall_sentiment`, not `market_sentiment`. Reading the wrong key here
    // dropped the 今日總結 line from every placeholder report — caught by
    // diffing against the Python on the live payload, not by any unit test.
    if let Some(s) = str_of(&summary, "overall_sentiment") {
        let c = pct(summary.get("confidence"));
        let suffix = if c != 0 { format!("（信心 {c}%）") } else { String::new() };
        lines.push(format!("今日總結：{}{suffix}", fmt_sentiment(s)));
    }
    let analyzed = i(summary.get("symbols_analyzed"));
    if analyzed != 0 {
        let bullish = i(summary.get("bullish_signals"));
        let bearish = i(summary.get("bearish_signals"));
        let neutral = analyzed - bullish - bearish;
        let n = if neutral > 0 {
            format!("｜中性 {neutral} 支")
        } else {
            String::new()
        };
        lines.push(format!("看漲 {bullish} 支｜看跌 {bearish} 支{n}"));
    }

    if let Some(events) = summary.get("key_events").and_then(|v| v.as_array()) {
        let real: Vec<&str> = events
            .iter()
            .filter_map(|e| e.as_str())
            .filter(|e| {
                !matches!(
                    *e,
                    "Market closed" | "Daily analysis complete" | "Tomorrow's outlook prepared"
                )
            })
            .collect();
        if !real.is_empty() {
            lines.push(String::new());
            for e in real.iter().take(3) {
                lines.push(format!("  • {e}"));
            }
        }
    }

    if let Some(signals) = data
        .get("high_confidence_signals")
        .and_then(|v| v.as_array())
        .filter(|a| !a.is_empty())
    {
        lines.push(String::new());
        lines.push("🎯 高信心訊號".into());
        for s in signals.iter().take(6) {
            let mut row = format!(
                "  • {} {} {}%",
                str_of(s, "symbol").unwrap_or(""),
                fmt_sentiment(str_of(s, "sentiment").unwrap_or("neutral")),
                pct(s.get("confidence"))
            );
            if let Some(r) = str_of(s, "reason").or_else(|| str_of(s, "reasoning")) {
                row += &format!(" — {}", clip(r, 80));
            }
            lines.push(row);
        }
    }

    if let Some(outlook) = data.get("tomorrow_outlook") {
        if let Some(s) = str_of(outlook, "sentiment").filter(|s| *s != "neutral") {
            let c = pct(outlook.get("confidence"));
            let suffix = if c != 0 {
                format!("（信心 {c}%）")
            } else {
                String::new()
            };
            lines.push(String::new());
            lines.push(format!("明日展望：{}{suffix}", fmt_sentiment(s)));
        }
    }

    lines.join("\n")
}

// ── intraday ─────────────────────────────────────────────────────────────────

pub fn format_intraday(data: &serde_json::Value, now: &str) -> String {
    let mut lines = vec![format!("📊 CCT 盤中報告｜{now}"), String::new()];

    let open = str_of(data, "market_status") == Some("open");
    lines.push(format!(
        "市場狀態：{}",
        if open { "開盤中 🟢" } else { "休市 ⚫" }
    ));

    let perf = data.get("current_performance");
    let sentiment = perf
        .and_then(|p| str_of(p, "market_sentiment"))
        .or_else(|| str_of(data, "sentiment_label"));
    if let Some(s) = sentiment {
        lines.push(format!("即時情緒：{}", fmt_sentiment(s)));
    }

    // The placeholder string is filtered out: "Morning predictions being
    // monitored" is what the route says when it is tracking nothing.
    if let Some(t) = perf
        .and_then(|p| str_of(p, "tracking_predictions"))
        .filter(|t| *t != "Morning predictions being monitored")
    {
        lines.push(format!("預測追蹤：{t}"));
    }

    let bullish = i(data.get("bullish_signals"));
    let bearish = i(data.get("bearish_signals"));
    if bullish != 0 || bearish != 0 {
        lines.push(String::new());
        lines.push(format!("看漲 {bullish} 支｜看跌 {bearish} 支"));
    }

    let signals = data
        .get("high_confidence_signals")
        .and_then(|v| v.as_array())
        .filter(|a| !a.is_empty());
    if let Some(list) = signals {
        lines.push(String::new());
        lines.push("🎯 高信心訊號".into());
        for s in list.iter().take(5) {
            lines.push(format!(
                "  • {} {} {}%",
                str_of(s, "symbol").unwrap_or(""),
                fmt_sentiment(str_of(s, "sentiment").unwrap_or("neutral")),
                pct(s.get("confidence"))
            ));
        }
    }

    if sentiment.is_none() && bullish == 0 && bearish == 0 && signals.is_none() {
        if let Some(m) = str_of(data, "message") {
            lines.push(format!("\n⏳ {m}"));
        }
    }

    lines.join("\n")
}

// ── weekly ───────────────────────────────────────────────────────────────────

pub fn format_weekly(data: &serde_json::Value) -> String {
    let mut lines = vec![
        format!("📊 CCT 週報｜{}", str_of(data, "week_start").unwrap_or("")),
        String::new(),
    ];

    // Top-level or nested under "report", as the route serves either.
    let report = data.get("report").unwrap_or(data);
    let overview = report.get("weekly_overview");

    if let Some(t) = overview.and_then(|o| str_of(o, "sentiment_trend")) {
        lines.push(format!(
            "本週趨勢：{}（平均信心 {}%）",
            fmt_sentiment(t),
            pct(overview.and_then(|o| o.get("average_confidence")))
        ));
    }

    let summary = report.get("weekly_summary");
    if let Some(r) = summary.and_then(|s| s.get("weekly_return")).and_then(|v| v.as_f64()) {
        let sign = if r >= 0.0 { "+" } else { "" };
        lines.push(format!("週平均回報：{sign}{r:.2}%"));
    }
    if let Some(v) = summary.and_then(|s| s.get("volatility")).and_then(|v| v.as_f64()) {
        lines.push(format!("波動率：{v:.2}%"));
    }

    if let Some(hs) = overview
        .and_then(|o| o.get("key_highlights"))
        .and_then(|v| v.as_array())
        .filter(|a| !a.is_empty())
    {
        lines.push(String::new());
        for h in hs.iter().take(3).filter_map(|h| h.as_str()) {
            lines.push(format!("  • {h}"));
        }
    }

    if let Some(days) = report
        .get("daily_breakdown")
        .and_then(|v| v.as_array())
        .filter(|a| !a.is_empty())
    {
        lines.push(String::new());
        lines.push("📅 每日紀錄".into());
        for d in days {
            lines.push(format!(
                "  {}  {}  訊號 {}",
                str_of(d, "date").unwrap_or(""),
                fmt_sentiment(str_of(d, "sentiment").unwrap_or("neutral")),
                i(d.get("signal_count"))
            ));
        }
    }

    let perf = report.get("performance_summary");
    let accuracy = perf.and_then(|p| p.get("accuracy_rate"));
    if accuracy.and_then(|v| v.as_f64()).unwrap_or(0.0) != 0.0 {
        lines.push(String::new());
        lines.push(format!(
            "準確率：{}%  總訊號：{}",
            pct(accuracy),
            i(perf.and_then(|p| p.get("total_signals")))
        ));
    }

    if let Some(s) = report
        .get("next_week_outlook")
        .and_then(|n| str_of(n, "sentiment"))
        .or_else(|| summary.and_then(|s| str_of(s, "next_week_sentiment")))
    {
        lines.push(String::new());
        lines.push(format!("下週展望：{}", fmt_sentiment(s)));
    }

    lines.join("\n")
}
