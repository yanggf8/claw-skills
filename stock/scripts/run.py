#!/usr/bin/env python3
"""Stock skill: fetch market indices (TWSE, HSI) and individual stock quotes."""
import argparse
import json
import os
import sys
import urllib.request

SKILLS_LIB = os.path.join(os.path.dirname(__file__), "..", "..", "lib")
sys.path.insert(0, os.path.abspath(SKILLS_LIB))
import telegram

# ── TWSE (Taiwan) ────────────────────────────────────────────────

def fetch_twse_index() -> dict:
    """Fetch TAIEX (加權指數) from TWSE."""
    url = "https://mis.twse.com.tw/stock/api/getStockInfo.jsp?ex_ch=tse_t00.tw&json=1"
    req = urllib.request.Request(url, headers={
        "Accept": "application/json",
        "User-Agent": "nullclaw/1.0",
    })
    with urllib.request.urlopen(req, timeout=15) as resp:
        return json.load(resp)


def fetch_twse_stock(symbol: str) -> dict:
    """Fetch individual TWSE stock quote."""
    ex_ch = f"tse_{symbol}.tw"
    url = f"https://mis.twse.com.tw/stock/api/getStockInfo.jsp?ex_ch={ex_ch}&json=1"
    req = urllib.request.Request(url, headers={
        "Accept": "application/json",
        "User-Agent": "nullclaw/1.0",
    })
    with urllib.request.urlopen(req, timeout=15) as resp:
        return json.load(resp)


def format_twse(data: dict) -> str:
    arr = data.get("msgArray", [])
    if not arr:
        return "[WARN: TWSE data unavailable]"
    s = arr[0]
    name = s.get("n", "?")
    price = s.get("z", "-")
    prev = s.get("y", "?")
    high = s.get("h", "?")
    low = s.get("l", "?")
    time = s.get("t", "?")
    date = s.get("d", "?")

    try:
        change = float(price) - float(prev)
        pct = change / float(prev) * 100
        sign = "+" if change >= 0 else ""
        change_str = f"{sign}{change:.2f} ({sign}{pct:.2f}%)"
    except (ValueError, ZeroDivisionError):
        change_str = ""

    line = f"📈 {name}：{price}"
    if change_str:
        line += f" {change_str}"
    line += f"\n   高 {high} / 低 {low}，{date} {time}"
    return line


# ── HSI (Hong Kong) ──────────────────────────────────────────────

def fetch_hsi() -> dict:
    """Fetch Hang Seng Index from HKEX."""
    url = "https://www1.hkex.com.hk/hkexwidget/data/getequityquote?sym=HSI&token=evLtsLsBNAUVTPxtGqVeG0MHLaWaqw5uh3fNH1bhmqr9UiDwNu9l5sJEMMGqrJuh&lang=tc&qid=&callback="
    req = urllib.request.Request(url, headers={
        "Accept": "application/json",
        "User-Agent": "nullclaw/1.0",
        "Referer": "https://www.hkex.com.hk/",
    })
    try:
        with urllib.request.urlopen(req, timeout=15) as resp:
            return json.load(resp)
    except Exception:
        return {}


def fetch_hsi_yahoo() -> str:
    """Fallback: fetch HSI from Yahoo Finance API."""
    url = "https://query1.finance.yahoo.com/v8/finance/chart/%5EHSI?interval=1d&range=5d"
    req = urllib.request.Request(url, headers={
        "Accept": "application/json",
        "User-Agent": "nullclaw/1.0",
    })
    with urllib.request.urlopen(req, timeout=15) as resp:
        data = json.load(resp)
    result = data.get("chart", {}).get("result", [{}])[0]
    meta = result.get("meta", {})
    price = meta.get("regularMarketPrice", "?")
    prev = meta.get("previousClose") or meta.get("chartPreviousClose", 0)
    try:
        change = float(price) - float(prev)
        pct = change / float(prev) * 100
        sign = "+" if change >= 0 else ""
        change_str = f"{sign}{change:.2f} ({sign}{pct:.2f}%)"
    except (ValueError, ZeroDivisionError):
        change_str = ""
    line = f"📈 恒生指數：{price}"
    if change_str:
        line += f" {change_str}"
    return line


def format_hsi(data: dict) -> str:
    quote = data.get("data", {}).get("quote", {})
    if not quote:
        return ""
    price = quote.get("last", "?")
    prev = quote.get("prvclose", "0")
    high = quote.get("high", "?")
    low = quote.get("low", "?")
    time = quote.get("updatetime", "?")

    try:
        change = float(str(price).replace(",", "")) - float(str(prev).replace(",", ""))
        pct = change / float(str(prev).replace(",", "")) * 100
        sign = "+" if change >= 0 else ""
        change_str = f"{sign}{change:.2f} ({sign}{pct:.2f}%)"
    except (ValueError, ZeroDivisionError):
        change_str = ""

    line = f"📈 恒生指數：{price}"
    if change_str:
        line += f" {change_str}"
    line += f"\n   高 {high} / 低 {low}，{time}"
    return line


# ── Main ─────────────────────────────────────────────────────────

def main():
    parser = argparse.ArgumentParser(description="Fetch stock/index data")
    parser.add_argument("--market", choices=["tw", "hk", "all"], default="all",
                        help="Market to query (default: all)")
    parser.add_argument("--symbol", default=None,
                        help="Individual stock symbol (e.g. 2330 for TSMC)")
    parser.add_argument("--deliver-to", dest="deliver_to", default=None, metavar="CHAT_ID")
    parser.add_argument("--account", dest="account", default="main")
    args = parser.parse_args()

    lines = []

    if args.symbol:
        try:
            data = fetch_twse_stock(args.symbol)
            lines.append(format_twse(data))
        except Exception as e:
            lines.append(f"[WARN: stock {args.symbol} unavailable - {e}]")
    else:
        if args.market in ("tw", "all"):
            try:
                data = fetch_twse_index()
                lines.append(format_twse(data))
            except Exception as e:
                lines.append(f"[WARN: TWSE index unavailable - {e}]")

        if args.market in ("hk", "all"):
            try:
                lines.append(fetch_hsi_yahoo())
            except Exception as e:
                try:
                    hsi_data = fetch_hsi()
                    hsi_line = format_hsi(hsi_data)
                    lines.append(hsi_line if hsi_line else f"[WARN: HSI unavailable]")
                except Exception as e2:
                    lines.append(f"[WARN: HSI unavailable - {e2}]")

    if not lines:
        lines.append("[WARN: no market data available]")

    output = "\n".join(lines)
    job_id = os.environ.get("NULLCLAW_JOB_ID")
    if job_id:
        output += f"\n\n`{job_id}`"
    if args.deliver_to:
        telegram.send(args.deliver_to, output, account=args.account)
    else:
        print(output)


if __name__ == "__main__":
    main()
