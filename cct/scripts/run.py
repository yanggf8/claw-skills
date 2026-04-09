#!/usr/bin/env python3
"""CCT skill: fetch 4-moment trading intelligence and deliver to Telegram."""
import argparse
import json
import os
import sys
import urllib.request
import urllib.error
from datetime import datetime, timezone

SKILLS_LIB = os.path.join(os.path.dirname(__file__), "..", "..", "lib")
sys.path.insert(0, os.path.abspath(SKILLS_LIB))
import telegram

CCT_BASE = "https://tft-trading-system.yanggf.workers.dev"
CONFIG_PATH = os.path.expanduser("~/.nullclaw/config.json")

SENTIMENT_EMOJI = {"bullish": "看漲 🟢", "bearish": "看跌 🔴", "neutral": "中性 ⚪"}


def load_api_key() -> str:
    try:
        with open(CONFIG_PATH) as f:
            return json.load(f).get("cct", {}).get("api_key", "yanggf")
    except Exception:
        return "yanggf"


def get(path: str) -> dict | None:
    url = f"{CCT_BASE}{path}"
    req = urllib.request.Request(
        url,
        headers={"X-API-Key": load_api_key(), "User-Agent": "nullclaw-cct/1.0"},
    )
    try:
        with urllib.request.urlopen(req, timeout=30) as resp:
            parsed = json.loads(resp.read().decode())
            if not parsed.get("success"):
                return None
            return parsed.get("data")
    except urllib.error.HTTPError as e:
        print(f"[WARN: CCT HTTP {e.code}] {e.read().decode(errors='replace')[:120]}", flush=True)
        return None
    except Exception as e:
        print(f"[WARN: CCT unavailable - {e}]", flush=True)
        return None


def fmt_sentiment(s: str) -> str:
    return SENTIMENT_EMOJI.get(s.lower(), s)


# ── formatters ────────────────────────────────────────────────────────────────

def format_pre_market(data: dict) -> str:
    today = datetime.now(timezone.utc).strftime("%Y-%m-%d")
    lines = [f"📊 CCT 盤前報告｜{today}", ""]

    # Overall market sentiment from signal aggregation
    overall = data.get("overall_sentiment", {})
    sentiment = overall.get("sentiment", data.get("market_sentiment", ""))
    confidence = overall.get("confidence", data.get("confidence", 0))
    analyzed = data.get("symbols_analyzed", 0) or len(data.get("trading_signals", {}))

    if sentiment:
        lines.append(f"市場情緒：{fmt_sentiment(sentiment)}（信心 {int(float(confidence) * 100)}%）")
    if analyzed:
        lines.append(f"分析標的：{analyzed} 支")
    lines.append("")

    # High-confidence signals
    signals = data.get("high_confidence_signals", [])
    if signals:
        lines.append("🎯 高信心訊號（≥70%）")
        for s in signals[:8]:
            sym = s.get("symbol", "")
            sent = fmt_sentiment(s.get("sentiment", "neutral"))
            conf = int(float(s.get("confidence", 0)) * 100)
            reason = s.get("reason", s.get("reasoning", ""))
            line = f"  • {sym} {sent} {conf}%"
            if reason:
                line += f" — {reason[:80]}"
            lines.append(line)
    else:
        msg = data.get("message", "")
        lines.append(f"⏳ {msg}" if msg else "今日尚無高信心訊號")

    return "\n".join(lines)


def format_intraday(data: dict) -> str:
    now = datetime.now(timezone.utc).strftime("%Y-%m-%d %H:%M UTC")
    lines = [f"📊 CCT 盤中報告｜{now}", ""]

    market_open = data.get("market_status", "") == "open"
    lines.append(f"市場狀態：{'開盤中 🟢' if market_open else '休市 ⚫'}")

    perf = data.get("current_performance", {})
    sentiment = perf.get("market_sentiment", data.get("sentiment_label", ""))
    if sentiment:
        lines.append(f"即時情緒：{fmt_sentiment(sentiment)}")

    tracking = perf.get("tracking_predictions", "")
    if tracking and tracking != "Morning predictions being monitored":
        lines.append(f"預測追蹤：{tracking}")

    # Signal breakdown if present
    bullish = data.get("bullish_signals", 0)
    bearish = data.get("bearish_signals", 0)
    if bullish or bearish:
        lines.append("")
        lines.append(f"看漲 {bullish} 支｜看跌 {bearish} 支")

    # High-confidence signals if present
    signals = data.get("high_confidence_signals", [])
    if signals:
        lines.append("")
        lines.append("🎯 高信心訊號")
        for s in signals[:5]:
            sym = s.get("symbol", "")
            sent = fmt_sentiment(s.get("sentiment", "neutral"))
            conf = int(float(s.get("confidence", 0)) * 100)
            lines.append(f"  • {sym} {sent} {conf}%")

    # Surface API message when no substantive data is available
    has_data = sentiment or bullish or bearish or signals
    if not has_data:
        msg = data.get("message", "")
        if msg:
            lines.append(f"\n⏳ {msg}")

    return "\n".join(lines)


def format_eod(data: dict) -> str:
    date = data.get("date", datetime.now(timezone.utc).strftime("%Y-%m-%d"))
    lines = [f"📊 CCT 收盤報告｜{date}", ""]

    summary = data.get("daily_summary", {})
    sentiment = summary.get("overall_sentiment", "")
    analyzed = summary.get("symbols_analyzed", 0)
    bullish = summary.get("bullish_signals", 0)
    bearish = summary.get("bearish_signals", 0)
    confidence = summary.get("confidence", 0)

    if sentiment:
        conf_str = f"（信心 {int(float(confidence) * 100)}%）" if confidence else ""
        lines.append(f"今日總結：{fmt_sentiment(sentiment)}{conf_str}")
    if analyzed:
        lines.append(f"分析標的：{analyzed} 支")
    if bullish or bearish:
        neutral = analyzed - bullish - bearish if analyzed else 0
        neutral_str = f"｜中性 {neutral} 支" if neutral > 0 else ""
        lines.append(f"看漲 {bullish} 支｜看跌 {bearish} 支{neutral_str}")

    # Key events
    events = summary.get("key_events", [])
    real_events = [e for e in events if e not in (
        "Market closed", "Daily analysis complete", "Tomorrow's outlook prepared"
    )]
    if real_events:
        lines.append("")
        for e in real_events[:3]:
            lines.append(f"  • {e}")

    # High-confidence signals
    signals = data.get("high_confidence_signals", [])
    if signals:
        lines.append("")
        lines.append("🎯 高信心訊號")
        for s in signals[:6]:
            sym = s.get("symbol", "")
            sent = fmt_sentiment(s.get("sentiment", "neutral"))
            conf = int(float(s.get("confidence", 0)) * 100)
            reason = s.get("reason", s.get("reasoning", ""))
            line = f"  • {sym} {sent} {conf}%"
            if reason:
                line += f" — {reason[:80]}"
            lines.append(line)

    # Tomorrow outlook
    outlook = data.get("tomorrow_outlook", {})
    outlook_sentiment = outlook.get("sentiment", "")
    outlook_conf = outlook.get("confidence", 0)
    if outlook_sentiment and outlook_sentiment != "neutral":
        lines.append("")
        conf_str = f"（信心 {int(float(outlook_conf) * 100)}%）" if outlook_conf else ""
        lines.append(f"明日展望：{fmt_sentiment(outlook_sentiment)}{conf_str}")

    # Surface API message when no substantive data is available
    has_data = sentiment or analyzed or signals
    if not has_data:
        msg = data.get("message", "")
        if msg:
            lines.append(f"\n⏳ {msg}")

    return "\n".join(lines)


def format_weekly(data: dict) -> str:
    week_start = data.get("week_start", "")
    lines = [f"📊 CCT 週報｜{week_start}", ""]

    report = data.get("report", data)  # top-level or nested under "report"
    overview = report.get("weekly_overview", {})
    sentiment_trend = overview.get("sentiment_trend", "")
    avg_conf = overview.get("average_confidence", 0)

    if sentiment_trend:
        lines.append(f"本週趨勢：{fmt_sentiment(sentiment_trend)}（平均信心 {int(float(avg_conf) * 100)}%）")

    weekly_summary = report.get("weekly_summary", {})
    weekly_return = weekly_summary.get("weekly_return", None)
    volatility = weekly_summary.get("volatility", None)
    if weekly_return is not None:
        sign = "+" if weekly_return >= 0 else ""
        lines.append(f"週平均回報：{sign}{weekly_return:.2f}%")
    if volatility is not None:
        lines.append(f"波動率：{volatility:.2f}%")

    highlights = overview.get("key_highlights", [])
    if highlights:
        lines.append("")
        for h in highlights[:3]:
            lines.append(f"  • {h}")

    # Daily breakdown
    breakdown = report.get("daily_breakdown", [])
    if breakdown:
        lines.append("")
        lines.append("📅 每日紀錄")
        for day in breakdown:
            date = day.get("date", "")
            day_sent = fmt_sentiment(day.get("sentiment", "neutral"))
            count = day.get("signal_count", 0)
            lines.append(f"  {date}  {day_sent}  訊號 {count}")

    # Performance summary
    perf = report.get("performance_summary", {})
    accuracy = perf.get("accuracy_rate", 0)
    total_signals = perf.get("total_signals", 0)
    if accuracy:
        lines.append("")
        lines.append(f"準確率：{int(float(accuracy) * 100)}%  總訊號：{total_signals}")

    # Next week outlook
    next_week = report.get("next_week_outlook", {})
    next_sentiment = next_week.get("sentiment", weekly_summary.get("next_week_sentiment", ""))
    if next_sentiment:
        lines.append("")
        lines.append(f"下週展望：{fmt_sentiment(next_sentiment)}")

    # Surface API message when no substantive data is available
    has_data = sentiment_trend or breakdown or weekly_return is not None
    if not has_data:
        msg = data.get("message", "")
        if msg:
            lines.append(f"\n⏳ {msg}")

    return "\n".join(lines)


# ── mode → endpoint + formatter ───────────────────────────────────────────────

MODES = {
    "pre-market": ("/api/v1/reports/pre-market", format_pre_market, "盤前報告"),
    "intraday":   ("/api/v1/reports/intraday",   format_intraday,   "盤中報告"),
    "eod":        ("/api/v1/reports/end-of-day",  format_eod,        "收盤報告"),
    "weekly":     ("/api/v1/reports/weekly",      format_weekly,     "週報"),
}


def main() -> None:
    parser = argparse.ArgumentParser(description="CCT 4-moment trading intelligence")
    parser.add_argument("--mode", required=True, choices=list(MODES),
                        help="pre-market | intraday | eod | weekly")
    parser.add_argument("--deliver-to", default=None, help="Telegram chat ID")
    parser.add_argument("--account", default="main", help="Telegram account name")
    args = parser.parse_args()

    endpoint, formatter, label = MODES[args.mode]
    data = get(endpoint)

    if data is None:
        msg = f"📭 CCT {label}尚未產生或暫時無法存取"
    else:
        msg = formatter(data)

    if args.deliver_to:
        ok = telegram.send(args.deliver_to, msg, account=args.account)
        if not ok:
            print(msg, flush=True)
    else:
        print(msg, flush=True)


if __name__ == "__main__":
    main()
