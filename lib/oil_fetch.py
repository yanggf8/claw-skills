"""Yahoo Finance fetch helpers for oil futures symbols."""
import json
from datetime import datetime, timezone
from urllib.parse import quote
import urllib.request


USER_AGENT = "nullclaw/1.0"
DEFAULT_TIMEOUT = 15


def build_chart_url(symbol: str, *, range_name: str) -> str:
    encoded = quote(symbol, safe="")
    return (
        "https://query1.finance.yahoo.com/v8/finance/chart/"
        f"{encoded}?interval=1d&range={range_name}"
    )


def parse_chart_response(payload: dict) -> list[tuple[str, float]]:
    chart = payload.get("chart", {})
    results = chart.get("result") or []
    if not results:
        return []

    result = results[0]
    timestamps = result.get("timestamp") or []
    quotes = (
        result.get("indicators", {})
        .get("quote", [{}])
    )
    closes = quotes[0].get("close") if quotes else None
    if not closes:
        return []

    rows: list[tuple[str, float]] = []
    for ts, close in zip(timestamps, closes):
        if close is None:
            continue
        day = datetime.fromtimestamp(ts, tz=timezone.utc).date().isoformat()
        rows.append((day, float(close)))
    return rows


def fetch_chart(symbol: str, *, range_name: str, timeout: int = DEFAULT_TIMEOUT) -> dict:
    url = build_chart_url(symbol, range_name=range_name)
    req = urllib.request.Request(
        url,
        headers={
            "Accept": "application/json",
            "User-Agent": USER_AGENT,
        },
    )
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        return json.load(resp)


def fetch_history(symbol: str, *, range_name: str = "1y", timeout: int = DEFAULT_TIMEOUT) -> list[tuple[str, float]]:
    payload = fetch_chart(symbol, range_name=range_name, timeout=timeout)
    return parse_chart_response(payload)


def fetch_latest(symbol: str, *, timeout: int = DEFAULT_TIMEOUT) -> tuple[str, float] | None:
    rows = fetch_history(symbol, range_name="5d", timeout=timeout)
    if not rows:
        return None
    return rows[-1]
