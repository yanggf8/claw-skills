#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import sys
from datetime import datetime, timedelta, timezone

SKILLS_LIB = os.path.join(os.path.dirname(__file__), "..", "..", "lib")
sys.path.insert(0, os.path.abspath(SKILLS_LIB))
import oil_fetch

try:
    from delivery import deliver_or_fail
    from trace_marker import emit_skill_status, emit_trace
except Exception:
    def deliver_or_fail(deliver_to, output, account="main"):
        print(output)
    def emit_skill_status(status):
        print(f"[skill-status:{status}]")
    def emit_trace():
        print(f"[trace:{os.environ.get('NULLCLAW_JOB_ID', 'manual')}]")


DEFAULT_CONFIG = Path.home() / ".nullclaw" / "skills" / "chipcon" / "config.json"


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    p = argparse.ArgumentParser(description="CHIPCON trend monitor")
    p.add_argument("--mode", choices=["deliver", "record"], default="deliver")
    p.add_argument("--config", default=str(DEFAULT_CONFIG))
    p.add_argument("--deliver-to", default=None, metavar="CHAT_ID")
    p.add_argument("--account", default="main")
    return p.parse_args(argv)


def default_events() -> list[str]:
    return [
        "NVDA / AVGO / AMD / MU guidance",
        "TSMC monthly revenue",
        "Microsoft / Amazon / Google / Meta capex guidance",
        "Export-control escalation",
        "SpaceX IPO / index-flow liquidity drain",
    ]


def load_config(path: Path) -> dict:
    if not path.exists():
        return {
            "symbols": {"SMH": "SMH", "QQQ": "QQQ", "SOXX": "SOXX"},
            "manual_events": default_events(),
        }
    with path.open("r", encoding="utf-8") as f:
        cfg = json.load(f)
    cfg.setdefault("symbols", {"SMH": "SMH", "QQQ": "QQQ", "SOXX": "SOXX"})
    cfg.setdefault("manual_events", default_events())
    return cfg


def as_rows(state: dict[str, list[list]], key: str) -> list[tuple[str, float]]:
    return [(str(day), float(close)) for day, close in state.get(key, [])]


def pct(a: float, b: float) -> float:
    return (a / b - 1.0) * 100.0


def ma(rows: list[tuple[str, float]], n: int) -> float | None:
    if len(rows) < n:
        return None
    return sum(close for _, close in rows[-n:]) / n


def ma_rising(rows: list[tuple[str, float]], n: int, lookback: int) -> bool | None:
    if len(rows) < n + lookback:
        return None
    current = ma(rows, n)
    past = ma(rows[:-lookback], n)
    if current is None or past is None:
        return None
    return current > past


def return_n(rows: list[tuple[str, float]], n: int) -> float | None:
    if len(rows) <= n:
        return None
    return pct(rows[-1][1], rows[-1 - n][1])


def consecutive_down(rows: list[tuple[str, float]]) -> int:
    count = 0
    for idx in range(len(rows) - 1, 0, -1):
        if rows[idx][1] < rows[idx - 1][1]:
            count += 1
        else:
            break
    return count


def classify(state: dict[str, list[list]]) -> tuple[str, dict]:
    smh = as_rows(state, "SMH")
    qqq = as_rows(state, "QQQ")
    soxx = as_rows(state, "SOXX")
    if len(smh) < 20:
        return "INSUFFICIENT_HISTORY", {"rows": len(smh)}

    current_day, current = smh[-1]
    ma20 = ma(smh, 20)
    ma50 = ma(smh, 50)
    rising20 = ma_rising(smh, 20, 5)
    smh5 = return_n(smh, 5)
    qqq5 = return_n(qqq, 5)
    soxx5 = return_n(soxx, 5)
    rel_qqq5 = None if smh5 is None or qqq5 is None else smh5 - qqq5
    rel_soxx5 = None if smh5 is None or soxx5 is None else smh5 - soxx5
    down_days = consecutive_down(smh)
    distance20 = None if ma20 is None else pct(current, ma20)
    distance50 = None if ma50 is None else pct(current, ma50)

    status = "OK"
    reasons: list[str] = []

    if ma50 is not None and current < ma50:
        status = "RED"
        reasons.append("SMH below 50DMA")
    if ma20 is not None and ma50 is not None and ma20 < ma50:
        status = "RED"
        reasons.append("20DMA below 50DMA")
    if rel_qqq5 is not None and rel_qqq5 <= -6.0:
        status = "RED"
        reasons.append("SMH underperformed QQQ by 6%+ over 5d")

    if status == "OK":
        if ma20 is not None and current < ma20 and rising20 is False:
            status = "ORANGE"
            reasons.append("SMH below 20DMA and 20DMA falling")
        if rel_qqq5 is not None and rel_qqq5 <= -4.0:
            status = "ORANGE"
            reasons.append("SMH underperformed QQQ by 4%+ over 5d")
        if down_days >= 3 and ma20 is not None and current < ma20:
            status = "ORANGE"
            reasons.append("3+ down days and below 20DMA")

    if status == "OK":
        if ma20 is not None and current < ma20:
            status = "YELLOW"
            reasons.append("SMH below 20DMA")
        elif rising20 is False:
            status = "YELLOW"
            reasons.append("20DMA no longer rising")
        elif rel_qqq5 is not None and rel_qqq5 <= -2.0:
            status = "YELLOW"
            reasons.append("SMH underperformed QQQ by 2%+ over 5d")

    if status == "OK" and distance20 is not None and distance20 >= 8.0 and down_days >= 1:
        status = "PROFIT_PROTECT"
        reasons.append("SMH still extended above 20DMA after a down day")

    details = {
        "day": current_day,
        "current": current,
        "ma20": ma20,
        "ma50": ma50,
        "rising20": rising20,
        "smh5": smh5,
        "qqq5": qqq5,
        "soxx5": soxx5,
        "rel_qqq5": rel_qqq5,
        "rel_soxx5": rel_soxx5,
        "down_days": down_days,
        "distance20": distance20,
        "distance50": distance50,
        "rows": len(smh),
        "reasons": reasons,
    }
    return status, details


def fmt_price(value: float | None) -> str:
    return "n/a" if value is None else f"{value:.2f}"


def fmt_pct(value: float | None) -> str:
    if value is None:
        return "n/a"
    prefix = "+" if value >= 0 else ""
    return f"{prefix}{value:.1f}%"


def format_message(status: str, details: dict, cfg: dict, warning: str | None = None) -> tuple[str, str]:
    lines = ["💾 CHIPCON 情報"]
    skill_status = "ok"
    if warning:
        lines.append(f"[WARN: {warning}]")
        skill_status = "degraded"
    lines.append(f"狀態：{status}")

    if status == "INSUFFICIENT_HISTORY":
        lines.append(f"SMH history rows: {details.get('rows', 0)} / 20 needed")
    else:
        ma20_dir = "rising" if details["rising20"] else "flat/down" if details["rising20"] is False else "n/a"
        lines.extend([
            f"SMH：{fmt_price(details['current'])} ({details['day']})",
            f"20DMA：{fmt_price(details['ma20'])} ({fmt_pct(details['distance20'])} vs 20DMA, {ma20_dir})",
            f"50DMA：{fmt_price(details['ma50'])} ({fmt_pct(details['distance50'])} vs 50DMA)",
            f"5日：SMH {fmt_pct(details['smh5'])} / QQQ {fmt_pct(details['qqq5'])} / SOXX {fmt_pct(details['soxx5'])}",
            f"相對：SMH-QQQ {fmt_pct(details['rel_qqq5'])} / SMH-SOXX {fmt_pct(details['rel_soxx5'])}",
            f"連跌：{details['down_days']} 日",
        ])
        if details["reasons"]:
            lines.append("原因：")
            lines.extend(f"- {reason}" for reason in details["reasons"])
        else:
            lines.append("原因：trend intact")

    lines.append("")
    lines.append("事件人工檢查：")
    for event in cfg.get("manual_events", default_events()):
        lines.append(f"- {event}")
    lines.append("")
    lines.append("SIGNAL-ONLY：這是動能觀測信號，不是交易指令。")
    return "\n".join(lines), skill_status


def update_state(cfg: dict) -> tuple[dict[str, list[list]], str | None]:
    symbols = cfg.get("symbols", {"SMH": "SMH", "QQQ": "QQQ", "SOXX": "SOXX"})
    warnings: list[str] = []
    state: dict[str, list[list]] = {}
    for key, symbol in symbols.items():
        sym = str(symbol).upper()
        try:
            rows = oil_fetch.fetch_history(sym, range_name="1y")
            rows = sorted(rows, key=lambda r: r[0])
            if not rows:
                warnings.append(f"yahoo {sym}: no rows")
            state[str(key)] = [[d, float(c)] for d, c in rows]
        except Exception as exc:
            warnings.append(f"yahoo fetch {sym}: {exc}")
            state[str(key)] = []
    if not state.get("SMH"):
        raise RuntimeError("; ".join(warnings) or "yahoo: no SMH history (primary symbol)")
    return state, "; ".join(warnings) if warnings else None


def record_line(status: str, details: dict, warning: str | None) -> str:
    now = datetime.now(timezone(timedelta(hours=8))).strftime("%Y-%m-%d %H:%M:%S CST")
    if status == "INSUFFICIENT_HISTORY":
        return f"{now} CHIPCON {status} rows={details.get('rows', 0)} warning={warning or '-'}"
    return (
        f"{now} CHIPCON {status} SMH={details['current']:.2f} "
        f"ma20={fmt_price(details['ma20'])} ma50={fmt_price(details['ma50'])} "
        f"rel_qqq5={fmt_pct(details['rel_qqq5'])} down={details['down_days']} "
        f"warning={warning or '-'}"
    )


def emit(message: str, status: str, args) -> None:
    # Send as plain text (parse_mode=None): the chipcon report is plain text —
    # status names carry underscores (e.g. INSUFFICIENT_HISTORY) and the Stooq
    # price-fetch WARN can carry anti-scraping JS garbage with unbalanced
    # backticks/brackets. Under Telegram legacy Markdown these break entity
    # parsing ("can't parse entities" HTTP 400 → delivery fails → degraded).
    # Nothing here is intentional Markdown, so plain text is both correct and
    # immune to any content. (Mirrors the news skill's markdown-unsafe → plain
    # fallback.)
    job_id = os.environ.get("NULLCLAW_JOB_ID")
    output = message
    if job_id:
        output += f"\n\n{job_id}"
    deliver_or_fail(args.deliver_to, output, account=args.account, parse_mode=None)
    emit_skill_status(status)
    emit_trace()


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    cfg = load_config(Path(args.config).expanduser())
    try:
        state, warning = update_state(cfg)
        status, details = classify(state)
        if args.mode == "record":
            path = Path("~/.nullclaw/chipcon-history.log").expanduser()
            path.parent.mkdir(parents=True, exist_ok=True)
            with path.open("a", encoding="utf-8") as f:
                f.write(record_line(status, details, warning) + "\n")
            emit_skill_status("degraded" if warning else "ok")
            emit_trace()
            return 0
        message, skill_status = format_message(status, details, cfg, warning)
        emit(message, skill_status, args)
        return 0
    except Exception as exc:
        print(f"CHIPCON failed: {exc}", file=sys.stderr)
        emit_skill_status("failed")
        emit_trace()
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
