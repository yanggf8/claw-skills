#!/usr/bin/env python3
"""Oilcon skill: fetch oil futures data and deliver or record a snapshot."""
import argparse
import os
import sys
from dataclasses import dataclass
from datetime import datetime, timedelta, timezone

SKILLS_LIB = os.path.join(os.path.dirname(__file__), "..", "..", "lib")
sys.path.insert(0, os.path.abspath(SKILLS_LIB))

from delivery import deliver_or_fail
import oil_fetch
import oil_store
from trace_marker import emit_skill_status, emit_trace


HISTORY_LOG = os.path.expanduser("~/.nullclaw/oilcon-history.log")
WINDOW_SIZE = 252
MIN_HISTORY_ROWS = 20
SYMBOLS = {
    "WTI": "CL=F",
    "Brent": "BZ=F",
    "HO": "HO=F",
}


@dataclass
class SymbolSnapshot:
    rows: list[tuple[str, float]] | None
    stale: bool = False


@dataclass
class Snapshot:
    symbols: dict[str, SymbolSnapshot]
    warning: str | None = None


def cst_now(with_seconds: bool = False) -> str:
    dt = datetime.now(timezone(timedelta(hours=8)))
    fmt = "%Y-%m-%d %H:%M:%S CST" if with_seconds else "%Y-%m-%d %H:%M"
    return dt.strftime(fmt)


def sign(value: float) -> int:
    if value > 0:
        return 1
    if value < 0:
        return -1
    return 0


def fmt_price(value: float) -> str:
    return f"${value:.2f}"


def fmt_pct(value: float) -> str:
    prefix = "+" if value >= 0 else ""
    return f"{prefix}{value:.1f}%"


def compute_change_pct(rows: list[tuple[str, float]]) -> float:
    if len(rows) < 2:
        raise ValueError("need at least 2 rows")
    prev = rows[-2][1]
    current = rows[-1][1]
    return (current - prev) / prev * 100.0


def compute_extremes(rows: list[tuple[str, float]]) -> dict[str, float | str | int]:
    if len(rows) < 2:
        raise ValueError("need at least 2 rows")

    current_day, current_close = rows[-1]
    high_index, high_row = max(enumerate(rows), key=lambda item: item[1][1])
    low_index, low_row = min(enumerate(rows), key=lambda item: item[1][1])

    return {
        "current_day": current_day,
        "current_close": current_close,
        "today_change_pct": compute_change_pct(rows),
        "high_day": high_row[0],
        "high_close": high_row[1],
        "days_since_high": len(rows) - 1 - high_index,
        "distance_from_high_pct": (current_close - high_row[1]) / high_row[1] * 100.0,
        "low_day": low_row[0],
        "low_close": low_row[1],
        "days_since_low": len(rows) - 1 - low_index,
        "distance_off_low_pct": (current_close - low_row[1]) / low_row[1] * 100.0,
    }


def confirmation_mark(confirm_change: float, wti_change: float) -> str:
    if confirm_change == 0 or wti_change == 0:
        return "–"
    return "✓" if sign(confirm_change) == sign(wti_change) else "✗"


def format_wti_line(symbol_snapshot: SymbolSnapshot) -> tuple[str, dict[str, float | str | int]]:
    if symbol_snapshot.rows is None:
        raise ValueError("WTI rows are required")

    wti = compute_extremes(symbol_snapshot.rows)
    line = f"WTI: {fmt_price(wti['current_close'])} ({fmt_pct(wti['today_change_pct'])})"
    if symbol_snapshot.stale:
        line += " (stale)"
    return line, wti


def format_confirmation_segment(
    label: str,
    symbol_snapshot: SymbolSnapshot,
    wti_change: float,
) -> str:
    if symbol_snapshot.rows is None:
        return f"{label} n/a"

    change = compute_change_pct(symbol_snapshot.rows)
    mark = confirmation_mark(change, wti_change)
    segment = f"{label} {mark} ({fmt_pct(change)})"
    if symbol_snapshot.stale:
        segment += " (stale)"
    return segment


def format_message(snapshot: Snapshot) -> tuple[str, str]:
    wti_snapshot = snapshot.symbols["WTI"]
    wti_line, wti = format_wti_line(wti_snapshot)
    brent_segment = format_confirmation_segment(
        "Brent",
        snapshot.symbols["Brent"],
        wti["today_change_pct"],
    )
    ho_segment = format_confirmation_segment(
        "HO",
        snapshot.symbols["HO"],
        wti["today_change_pct"],
    )

    lines = ["🛢️ OILCON 情報"]
    status = "ok"
    if snapshot.warning:
        lines.append(f"[WARN: {snapshot.warning}]")
        status = "degraded"

    lines.extend(
        [
            wti_line,
            f"  高 {fmt_price(wti['high_close'])} ({wti['high_day']}, {wti['days_since_high']}日前, {fmt_pct(wti['distance_from_high_pct'])})",
            f"  低 {fmt_price(wti['low_close'])} ({wti['low_day']}, {wti['days_since_low']}日前, {fmt_pct(wti['distance_off_low_pct'])} 離低點)",
            f"確認：{brent_segment}   {ho_segment}",
            f"更新：{cst_now()}",
        ]
    )
    return "\n".join(lines), status


def format_record_line(snapshot: Snapshot) -> str:
    if snapshot.warning:
        raise ValueError("record mode requires fresh data")

    wti_snapshot = snapshot.symbols["WTI"]
    brent_snapshot = snapshot.symbols["Brent"]
    ho_snapshot = snapshot.symbols["HO"]
    if wti_snapshot.stale or brent_snapshot.stale or ho_snapshot.stale:
        raise ValueError("record mode requires non-stale data")
    if wti_snapshot.rows is None or brent_snapshot.rows is None or ho_snapshot.rows is None:
        raise ValueError("record mode requires complete confirmation data")

    wti = compute_extremes(wti_snapshot.rows)
    brent_change = compute_change_pct(brent_snapshot.rows)
    ho_change = compute_change_pct(ho_snapshot.rows)
    return (
        f"{cst_now(with_seconds=True)}  "
        f"WTI {wti['current_close']:.2f}  "
        f"high {wti['high_close']:.2f}@{wti['high_day']} ({fmt_pct(wti['distance_from_high_pct'])})  "
        f"low {wti['low_close']:.2f}@{wti['low_day']} ({fmt_pct(wti['distance_off_low_pct'])})  "
        f"BZ {fmt_pct(brent_change)} HO {fmt_pct(ho_change)}"
    )


def build_symbol_snapshot(conn, label: str, symbol: str) -> SymbolSnapshot:
    first_run = oil_store.needs_backfill(conn, symbol)
    if first_run:
        try:
            history_rows = oil_fetch.fetch_history(symbol)
        except Exception as exc:
            raise ValueError(f"history fetch failed for {label} - {exc}") from exc
        oil_store.insert_many(conn, symbol, history_rows)

    latest_failed = False
    try:
        latest_row = oil_fetch.fetch_latest(symbol)
    except Exception:
        latest_row = None
        latest_failed = True
    if latest_row is not None:
        oil_store.upsert(conn, symbol, latest_row[0], latest_row[1])
    else:
        latest_failed = True

    rows = oil_store.window(conn, symbol, WINDOW_SIZE)
    if len(rows) < MIN_HISTORY_ROWS:
        if label == "WTI":
            raise ValueError(f"insufficient WTI history ({len(rows)} rows)")
        return SymbolSnapshot(rows=None)

    return SymbolSnapshot(rows=rows, stale=latest_failed)


def build_snapshot() -> Snapshot:
    try:
        conn = oil_store.connect_from_env()
    except oil_store.MissingCredentialsError as exc:
        return Snapshot(symbols={}, warning=str(exc))
    except Exception as exc:
        return Snapshot(symbols={}, warning=f"turso unavailable - {exc}")

    try:
        oil_store.ensure_schema(conn)
        symbols: dict[str, SymbolSnapshot] = {}
        for label, symbol in SYMBOLS.items():
            try:
                symbols[label] = build_symbol_snapshot(conn, label, symbol)
            except Exception as exc:
                return Snapshot(symbols={}, warning=str(exc))
        return Snapshot(symbols=symbols)
    finally:
        conn.close()


def emit_and_exit(message: str, status: str, args) -> None:
    job_id = os.environ.get("NULLCLAW_JOB_ID")
    output = message
    if job_id:
        output += f"\n\n`{job_id}`"

    deliver_or_fail(args.deliver_to, output, account=args.account)
    emit_skill_status(status)
    emit_trace()


def main():
    parser = argparse.ArgumentParser(description="Fetch oil futures snapshot")
    parser.add_argument(
        "--mode",
        choices=["deliver", "record"],
        default="deliver",
        help="deliver: print/send formatted output; record: append to history log",
    )
    parser.add_argument("--deliver-to", dest="deliver_to", default=None, metavar="CHAT_ID")
    parser.add_argument("--account", default="main", help="Telegram bot account name")
    args = parser.parse_args()

    snapshot = build_snapshot()
    if snapshot.warning:
        if args.mode == "deliver":
            message = f"🛢️ OILCON 情報\n[WARN: {snapshot.warning}]\n更新：{cst_now()}"
            emit_and_exit(message, "degraded", args)
            return
        print(f"[ERROR: {snapshot.warning}]", file=sys.stderr)
        sys.exit(1)

    if args.mode == "deliver":
        message, status = format_message(snapshot)
        emit_and_exit(message, status, args)
        return

    try:
        line = format_record_line(snapshot)
        with open(HISTORY_LOG, "a", encoding="utf-8") as handle:
            handle.write(line + "\n")
    except Exception as exc:
        print(f"[ERROR: could not write history log - {exc}]", file=sys.stderr)
        sys.exit(1)

    emit_skill_status("ok")
    emit_trace()


if __name__ == "__main__":
    main()
