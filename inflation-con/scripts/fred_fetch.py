"""FRED fetch helper for inflation-con.

Uses FRED's public `fredgraph.csv` endpoint, which returns a plain CSV of
(date, value) rows and requires NO API key — unlike the JSON web-service API,
which demands a 32-char key. CSV is also the right transport for the
agent-first / NO-JSON project rule: this module is the transport adapter, and
JSON never enters the data model. The rest of the skill sees only
list[tuple[str, float]] rows.

Series used (monthly unless noted):
  PCEPILFE  core PCE price index          (Fed's primary gauge)
  PCEPI     headline PCE price index      (context)
  CPILFESL  core CPI                       (confirmation)
  CPIAUCSL  headline CPI                   (context)
  T10YIE    10-yr breakeven inflation      (daily, market-priced)
  DFII10    10-yr TIPS real yield          (daily, context)
  DGS10     10-yr Treasury yield           (daily, context)
"""
from __future__ import annotations

import csv
import io
import urllib.request

USER_AGENT = "nullclaw/1.0"
DEFAULT_TIMEOUT = 20


def build_csv_url(series_id: str) -> str:
    return f"https://fred.stlouisfed.org/graph/fredgraph.csv?id={series_id}"


def parse_csv(text: str) -> list[tuple[str, float]]:
    """Parse a fredgraph.csv body into (date, value) rows.

    FRED marks missing observations with a lone ".". Those rows are skipped,
    so the returned series contains only real numeric observations, oldest
    first (FRED's native order).
    """
    rows: list[tuple[str, float]] = []
    reader = csv.reader(io.StringIO(text))
    header = next(reader, None)  # skip the "observation_date,<SERIES>" header
    if header is None:
        return rows
    for row in reader:
        if len(row) < 2:
            continue
        day, raw = row[0].strip(), row[1].strip()
        if raw in (".", ""):
            continue
        try:
            rows.append((day, float(raw)))
        except ValueError:
            continue
    return rows


def fetch_series(series_id: str, *, timeout: int = DEFAULT_TIMEOUT) -> list[tuple[str, float]]:
    """Fetch one FRED series as (date, value) rows, oldest first. No API key."""
    url = build_csv_url(series_id)
    req = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        text = resp.read().decode("utf-8")
    return parse_csv(text)
